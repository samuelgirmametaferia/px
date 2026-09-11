# Product releases

Product tags use `vMAJOR.MINOR.PATCH`; registry tags use `registry-*`.
Registry publication explicitly uses `--latest=false` so metadata does not
replace the product's latest-release link.

The product workflow builds native Linux x86_64 and ARM64 executables, runs
Rust checks and installer regressions, and creates a **draft** release for a
version tag. A manual workflow run builds artifacts without creating a release.
The tag must match the version in `Cargo.toml`.

The draft contains:

- `px-linux-x86_64` and `px-linux-aarch64`;
- `px`, an x86_64 compatibility asset for older self-updaters;
- `install.sh`, defaulting to that exact version;
- `SHA256SUMS`, checksums for the downloadable files.

The installer accepts `PX_VERSION=v…` and `PX_INSTALL_DIR=/your/path`.
It supports curl or wget, checks that the downloaded executable runs and reports
the expected version, and stages replacement beside the destination. It does
not currently verify SHA256SUMS automatically. The version check verifies
compatibility, not publisher authenticity.

Before publishing a draft, inspect the workflow results and release notes,
verify downloads on the supported architectures, and test installing into a
fresh user directory and upgrading an existing installation. Test `px doctor`
and a dry-run install on each supported distro. Native builds do not by
themselves prove compatibility with every distro or older glibc version.

Publish the reviewed product release as Latest, then update the README's
prebuilt installation instructions to the verified release. Do not advertise
an ARM binary until its release asset is available. Existing v3.1.0 releases
only provide the legacy x86_64 `px` asset.
