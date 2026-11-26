## Contribution Track
- [ ] **Track 1 — Core Application (`src/`)**
- [ ] **Track 2 — Plugin / SDK (`plugins/`, `docs/PLUGIN_SDK.md`)**
- [ ] Documentation / CI / Maintenance

## Summary
Describe what changed and why.

## Verification Checklist
List only commands and manual checks you actually ran after your final edit:
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --locked`
- [ ] `cargo build --release --locked`
- [ ] Plugin script syntax / standalone output checked (if modifying `plugins/`)
- [ ] Visual UI check in Windows Light & Dark mode (if modifying UI)

## Related Issues
Closes #
