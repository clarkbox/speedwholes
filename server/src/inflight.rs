//! Coalesces identical in-flight resize requests.
//!
//! The first request for a key becomes the leader and does the work. Later
//! requests for that key wait for the leader's result instead of fetching and
//! decoding the same image again. The map lock is not held across an await.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;

use crate::error::AppError;

/// A finished resize that followers can serve as-is.
#[derive(Debug)]
pub struct SharedImage {
    pub data: Vec<u8>,
    pub output_extension: &'static str,
    pub original_extension: &'static str,
}

/// Why a follower was not given an image.
#[derive(Debug)]
pub enum WaitError {
    /// The leader finished with this error. Followers return it and do not retry.
    Failed(Arc<AppError>),
    /// The leader's task ended without a result. The key is free.
    Cancelled,
}

#[derive(Clone)]
enum SlotValue {
    Pending,
    Ready(Arc<SharedImage>),
    Failed(Arc<AppError>),
}

type Slot = watch::Receiver<SlotValue>;

#[derive(Default)]
pub struct Inflight {
    map: Mutex<HashMap<String, Slot>>,
}

pub enum Join {
    Leader(Leader),
    Follower(Follower),
}

/// Held by the request that does the work. Dropping it without [`Leader::publish`]
/// or [`Leader::fail`] wakes followers with [`WaitError::Cancelled`].
pub struct Leader {
    key: String,
    owner: Arc<Inflight>,
    tx: watch::Sender<SlotValue>,
}

pub struct Follower {
    rx: Slot,
}

impl Inflight {
    pub fn join(self: &Arc<Self>, key: &str) -> Join {
        let mut map = self.map.lock();
        if let Some(rx) = map.get(key) {
            return Join::Follower(Follower { rx: rx.clone() });
        }
        let (tx, rx) = watch::channel(SlotValue::Pending);
        map.insert(key.to_string(), rx);
        Join::Leader(Leader {
            key: key.to_string(),
            owner: Arc::clone(self),
            tx,
        })
    }
}

impl Leader {
    /// Hand the result to every follower. The key stays claimed until this
    /// leader is dropped, so a request that arrives before the cache write
    /// finishes still receives this result.
    pub fn publish(&self, image: Arc<SharedImage>) {
        let _ = self.tx.send(SlotValue::Ready(image));
    }

    /// Hand the same error to every follower, then release the key.
    pub fn fail(self, err: Arc<AppError>) {
        let _ = self.tx.send(SlotValue::Failed(err));
    }
}

impl Drop for Leader {
    fn drop(&mut self) {
        self.owner.map.lock().remove(&self.key);
    }
}

impl Follower {
    /// The leader's result. `Err(Cancelled)` means the leader disappeared
    /// and the caller should try to lead. `Err(Failed)` is terminal.
    pub async fn wait(mut self) -> Result<Arc<SharedImage>, WaitError> {
        loop {
            match &*self.rx.borrow() {
                SlotValue::Ready(image) => return Ok(Arc::clone(image)),
                SlotValue::Failed(err) => return Err(WaitError::Failed(Arc::clone(err))),
                SlotValue::Pending => {}
            }
            if self.rx.changed().await.is_err() {
                return match &*self.rx.borrow() {
                    SlotValue::Ready(image) => Ok(Arc::clone(image)),
                    SlotValue::Failed(err) => Err(WaitError::Failed(Arc::clone(err))),
                    SlotValue::Pending => Err(WaitError::Cancelled),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(byte: u8) -> Arc<SharedImage> {
        Arc::new(SharedImage {
            data: vec![byte; 4],
            output_extension: "jpg",
            original_extension: "jpg",
        })
    }

    #[tokio::test]
    async fn followers_receive_the_leaders_bytes() {
        let inflight = Arc::new(Inflight::default());
        let Join::Leader(leader) = inflight.join("k") else {
            panic!("first join must lead");
        };
        let mut waiters = Vec::new();
        for _ in 0..3 {
            let Join::Follower(follower) = inflight.join("k") else {
                panic!("later join must follow");
            };
            waiters.push(tokio::spawn(follower.wait()));
        }
        leader.publish(image(7));
        drop(leader);
        for waiter in waiters {
            assert_eq!(waiter.await.unwrap().unwrap().data, vec![7; 4]);
        }
        assert!(matches!(inflight.join("k"), Join::Leader(_)));
    }

    #[tokio::test]
    async fn a_published_result_survives_the_leader_dropping() {
        let inflight = Arc::new(Inflight::default());
        let Join::Leader(leader) = inflight.join("k") else {
            panic!("first join must lead");
        };
        let Join::Follower(early) = inflight.join("k") else {
            panic!("second join must follow");
        };
        leader.publish(image(3));
        let Join::Follower(late) = inflight.join("k") else {
            panic!("join before drop must follow");
        };
        drop(leader);
        assert_eq!(early.wait().await.unwrap().data, vec![3; 4]);
        assert_eq!(late.wait().await.unwrap().data, vec![3; 4]);
    }

    #[tokio::test]
    async fn a_failed_leader_shares_the_error_and_releases_the_key() {
        let inflight = Arc::new(Inflight::default());
        let Join::Leader(leader) = inflight.join("k") else {
            panic!("first join must lead");
        };
        let Join::Follower(follower) = inflight.join("k") else {
            panic!("second join must follow");
        };
        let err = Arc::new(AppError::DecodeError("not an image".to_string()));
        leader.fail(Arc::clone(&err));
        match follower.wait().await {
            Err(WaitError::Failed(got)) => assert_eq!(got.to_string(), err.to_string()),
            other => panic!("expected the shared error, got {other:?}"),
        }
        let Join::Leader(next) = inflight.join("k") else {
            panic!("key must be free after the failure");
        };
        next.publish(image(1));
        drop(next);
    }

    #[tokio::test]
    async fn a_dropped_leader_cancels_waiters_so_one_can_take_over() {
        let inflight = Arc::new(Inflight::default());
        let Join::Leader(leader) = inflight.join("k") else {
            panic!("first join must lead");
        };
        let Join::Follower(follower) = inflight.join("k") else {
            panic!("second join must follow");
        };
        drop(leader);
        assert!(matches!(follower.wait().await, Err(WaitError::Cancelled)));
        let Join::Leader(successor) = inflight.join("k") else {
            panic!("waiter must be able to lead");
        };
        let Join::Follower(third) = inflight.join("k") else {
            panic!("a later request follows the successor");
        };
        successor.publish(image(9));
        drop(successor);
        assert_eq!(third.wait().await.unwrap().data, vec![9; 4]);
    }

    #[tokio::test]
    async fn aborting_the_leader_task_cancels_its_waiters() {
        let inflight = Arc::new(Inflight::default());
        let (started, start_rx) = tokio::sync::oneshot::channel();
        let table = Arc::clone(&inflight);
        let leader_task = tokio::spawn(async move {
            let Join::Leader(_leader) = table.join("k") else {
                panic!("first join must lead");
            };
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        start_rx.await.expect("leader started");
        let Join::Follower(follower) = inflight.join("k") else {
            panic!("second join must follow");
        };
        leader_task.abort();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), follower.wait())
            .await
            .expect("cancellation must wake the waiter");
        assert!(matches!(result, Err(WaitError::Cancelled)));
        assert!(matches!(inflight.join("k"), Join::Leader(_)));
    }

    #[tokio::test]
    async fn distinct_keys_do_not_share_a_result() {
        let inflight = Arc::new(Inflight::default());
        let Join::Leader(first) = inflight.join("a") else {
            panic!("a leads");
        };
        let Join::Leader(second) = inflight.join("b") else {
            panic!("b leads");
        };
        let Join::Follower(follow_a) = inflight.join("a") else {
            panic!("a follows");
        };
        first.publish(image(1));
        second.publish(image(2));
        drop(first);
        drop(second);
        assert_eq!(follow_a.wait().await.unwrap().data, vec![1; 4]);
    }
}
