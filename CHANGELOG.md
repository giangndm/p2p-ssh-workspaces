# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/giangndm/ssh-over-iroh/compare/v0.1.0...v0.1.1) - 2026-10-14

### Added

- *(config)* log unified git-diff changes and add CI workflow
- add bash install script for latest release binary

### Fixed

- *(ci)* ensure release job runs when release-plz-release is skipped
- *(shared)* replace match with ? operator for clippy

### Other

- update repository URL to giangndm/ssh-over-iroh
- *(release)* automate version releases with release-plz
- expand installation and quick-start guide in README
- require human approval before merging pull requests
- require rebase or squash when merging pull requests
