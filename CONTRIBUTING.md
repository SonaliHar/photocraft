# Contributing to ParodyShop™

Thank you for your interest in contributing to ParodyShop! Before you begin, please make sure you have an active
**ParodyShop Contributor Seat**:

| Seat | Price | Includes |
|---|---|---|
| Contributor Lite | $19.99/mo | Opening issues (read-only) |
| Contributor | $49.99/mo | Opening pull requests, billed per line of code (deletions billed double) |
| Contributor Pro Max | $199.99/mo | Your PR is reviewed by a human, eventually |
| Maintainer | Contact Sales | The next available call is in March 2031 |

Commits must be signed. And notarized. And witnessed by two (2) Enterprise customers.

---

Just kidding. Contributions are free and welcome, especially new jokes. The rules:

1. **Never collect real payment details.** No card fields, no links to real payment or checkout pages, nothing that could
   take a real person's money. The joke is that you *can't* pay; the moment you could, it stops being a parody and starts
   being a scam.
2. **No real company logos or branding,** and nothing that could be mistaken for a real product's dialogs.
3. **Punch up.** The target is pricing pages, dark patterns and subscription fatigue, never the user.
4. **Keep the editor underneath working.** `PARODYSHOP_OPEN_SOURCE=1` must still give you the full editor, and the
   paywall stays off in `Services::default()` so the upstream test suite keeps testing real features.

Where things go:

- New paywall pitches, plans and payment-decline messages: `crates/ui-egui/src/paywall.rs`.
- New SaaS experiences (banners, pop-ups, meters, onboarding): `crates/ui-egui/src/saas.rs`.

Before opening a PR:

```sh
cargo test -p photocraft-ui-egui --lib -- paywall saas
cargo clippy -p photocraft-ui-egui --all-targets -- -D warnings
```

For everything about the editor itself (architecture, the never-panic rules, tests), see
[AGENTS.md](AGENTS.md) and [docs/contributing.md](docs/contributing.md), inherited from
[PhotoCraft](https://github.com/storytold/photocraft).
