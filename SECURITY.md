# Security

## Supported versions

Security fixes land on the latest `main` branch. Older commits are not a supported release line.

## Reporting a vulnerability

Report vulnerabilities through GitHub private vulnerability reporting: the repository Security tab, then "Report a vulnerability". The repository owner has to enable that feature before the form is available.

Do not open a public GitHub issue for a suspected vulnerability.

## Disclosure

Please allow 90 days from a report before discussing the details in public. That window is for confirming the report and publishing a fix.

## Scope

In scope:

- the image server in `server/`
- the admin UI in `www/`
- the install, deploy, and development scripts

A deployment that turns on `allow_all_sources` or `allow_private_networks` is choosing that behaviour. Those switches are not vulnerabilities in the software.
