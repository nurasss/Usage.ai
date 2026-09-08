# Intel installation checklist

- [x] Host reports `x86_64` and Intel Core i5.
- [x] Rust host target is `x86_64-apple-darwin`.
- [x] Bundle identifier is `com.nurasss.usageai`.
- [x] Minimum macOS metadata is 13.0.
- [x] App uses accessory activation policy (no required Dock icon).
- [x] Updater is absent until a signed channel is configured.
- [x] `cargo test --workspace` passes (32 tests).
- [x] `npm run check`, `npm test`, and production build pass.
- [x] `.app` and Intel `.dmg` build.
- [x] `file` confirms the packaged executable is x86_64.
- [ ] Tray click/right-click, outside-click, `Esc`, `⌘R`, `⌘,`, and `⌃⌥U` are smoke-tested.
- [ ] Launch-at-login and denied notification permission are smoke-tested.
- [ ] Sleep/wake and offline cache behavior are smoke-tested.
- [ ] Quotas are manually compared with authoritative client UI at the same moment/account.
- [ ] Cached panel p95, cold start, idle CPU, and memory are measured.

