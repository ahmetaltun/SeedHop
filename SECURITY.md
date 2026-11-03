# Security policy

Security fixes target the current development branch and the latest stable
release. Include the exact app version and installation source in a report.

## Reporting a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/ahmetaltun/SeedHop/security/advisories/new)
when it is available. If private reporting is unavailable, open an issue asking
for a private reporting channel without publishing exploit details or personal
data. Do not attach a real torrent library, tracker passkey, VPN credentials,
download history, or unredacted logs to a public issue.

A useful report includes the affected version, macOS version, expected and actual
behavior, a minimal reproduction using synthetic data, and the security impact.
For routing problems, describe which traffic escaped the intended interface and
how it was measured. Use a controlled swarm and redact packet captures before
sharing them.

## Handling reports

The review process is to reproduce the issue privately, identify affected
versions and boundaries, prepare a fix with a regression test, and coordinate
release and disclosure with the reporter. Reports involving leaked credentials
or compromised releases also require an assessment of revocation, key rotation,
and user notification. No bounty or response-time guarantee is implied.

## Protection limits

Ordinary BitTorrent participation is observable by peers and trackers. Peer
encryption does not provide anonymity, and quarantine does not establish that a
download is safe. See [the hardening checkpoint](docs/SECURITY-HARDENING.md) for
implemented boundaries, executed tests, and remaining acceptance gates.
