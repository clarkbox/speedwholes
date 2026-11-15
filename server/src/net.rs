//! DNS resolution that drops private and reserved addresses before connect.

use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::source_filter::is_forbidden_ip;

/// Returned when every resolved address was private or reserved.
#[derive(Debug)]
pub struct ForbiddenAddress {
    host: String,
}

impl std::fmt::Display for ForbiddenAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no allowed address for {}", self.host)
    }
}

impl Error for ForbiddenAddress {}

/// Resolves a name with the system resolver and drops forbidden addresses.
#[derive(Debug, Clone, Copy)]
pub struct FilteringResolver {
    pub allow_private: bool,
}

impl FilteringResolver {
    pub fn shared(allow_private: bool) -> Arc<Self> {
        Arc::new(Self { allow_private })
    }
}

impl Resolve for FilteringResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_private = self.allow_private;
        let host = name.as_str().to_string();
        Box::pin(resolve_filtered(host, allow_private))
    }
}

async fn resolve_filtered(
    host: String,
    allow_private: bool,
) -> Result<Addrs, Box<dyn Error + Send + Sync>> {
    let looked_up = tokio::net::lookup_host((host.as_str(), 0u16))
        .await
        .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;
    let addrs: Vec<SocketAddr> = looked_up
        .filter(|addr| allow_private || !is_forbidden_ip(addr.ip()))
        .collect();
    if addrs.is_empty() {
        return Err(Box::new(ForbiddenAddress { host }) as Box<dyn Error + Send + Sync>);
    }
    Ok(Box::new(addrs.into_iter()) as Addrs)
}

/// True when `err` or any error in its `source()` chain is a [`ForbiddenAddress`].
pub fn is_forbidden_address(err: &(dyn Error + 'static)) -> bool {
    if err.downcast_ref::<ForbiddenAddress>().is_some() {
        return true;
    }
    let mut current = err.source();
    while let Some(source) = current {
        if source.downcast_ref::<ForbiddenAddress>().is_some() {
            return true;
        }
        current = source.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn localhost_is_rejected_unless_private_is_allowed() {
        let blocked = FilteringResolver {
            allow_private: false,
        };
        let name: Name = "localhost".parse().expect("name");
        assert!(blocked.resolve(name).await.is_err());

        let allowed = FilteringResolver {
            allow_private: true,
        };
        let name: Name = "localhost".parse().expect("name");
        let addrs = allowed.resolve(name).await.expect("localhost resolves");
        assert!(addrs.count() > 0);
    }
}
