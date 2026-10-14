<h1 align="center">ParodyShop™</h1>

<p align="center">
  <b>The open-source image editor where every feature is a Premium™ feature.</b><br>
  100% Rust. 100% open source. 0% of features included in your plan.
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="Features: paywalled" src="https://img.shields.io/badge/features-paywalled-critical?style=flat-square">
  <img alt="Price: surging" src="https://img.shields.io/badge/price-surging%20%E2%86%97-orange?style=flat-square">
  <img alt="Coverage: requires Premium" src="https://img.shields.io/badge/coverage-requires%20Premium-lightgrey?style=flat-square">
  <img alt="Uptime of your wallet: 0%" src="https://img.shields.io/badge/wallet%20uptime-0%25-red?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
</p>

<p align="center">
  <img src="docs/images/parodyshop-paywall.png" alt="ParodyShop showing a subscription offer for Gaussian Blur: three plans from Basic (the feature's icon, not the feature) to Ultimate Enterprise Platinum, with Subscribe, Watch an ad and No thanks buttons" width="100%">
  <br>
  <sub>Trying to use Gaussian Blur. The free trial lets you look at a screenshot of the features you don't have.</sub>
</p>

> [!WARNING]
> **This is a parody.** ParodyShop is a joke fork of [PhotoCraft](https://github.com/storytold/photocraft), an open-source
> image editor written in Rust. It is not affiliated with, endorsed by or sponsored by Adobe, the PhotoCraft maintainers,
> or anyone with a pricing page. **It never asks for payment details and never takes money.** Every "payment" is declined.

## What is this?

You know how image editors used to be software you bought, and then became software you rent, and then became software
that rents *you*? ParodyShop finishes the journey.

Underneath, it's a complete native image editor: layers, masks, adjustment layers, filters, type, vectors, brushes and
real PSD files. On top, every single one of those features is behind a subscription offer you cannot complete.

## Features™

| Feature | Free tier | Premium |
|---|---|---|
| Hand tool | ✅ So you can wave goodbye to your money | ✅ |
| Zoom tool | ✅ So you can get a closer look at what you're missing | ✅ |
| Every other tool | ❌ | ❌ Payment declined |
| Every menu item | ❌ | ❌ Payment declined |
| Undo | ❌ Regret is a luxury good ($999.99/mo) | ❌ |
| Redo | ❌ Billed per undo of the undo ($1,499.99/mo) | ❌ |
| File › Exit | ❌ Requires ParodyShop Exit+ | ❌ The window's close button is still free. For now. |
| Fonts | ❌ Licensed per letter. Vowels cost extra. | ❌ |
| Filters | ❌ Now AI-powered™: 12 Generative Credits each. You have 0. | ❌ |
| Help | ❌ Enterprise support contract required (response time: 6–8 business years) | ❌ |

### The subscription experience

- **Three plans for every feature.** *Basic* includes the feature's icon, not the feature. *Pro* includes the feature on
  weekdays, outside business hours. *Ultimate Enterprise Platinum* is the most popular, because we say so.
- **Surge pricing.** Each time you try a feature again, it costs 15% more. Persistence is a premium trait.
- **Payment processing.** "Contacting Creative Clout™…", "Checking your credit score…", "Selling your browsing history to
  cover the transaction fee…". Then: *"Card declined: ParodyShop does not accept real money."*
- **Watch an ad to unlock.** A five-second ad for ParodyShop Premium™ (now with even more paywalls, in 4K!). Your ad
  revenue of $0.0003 is credited to your account. Remaining balance: still a lot.
- **A "No thanks" button that argues back.** *"Real artists pay. Are you sure you want to keep your money?"* Declining a
  subscription you don't have incurs an early termination fee of $84.99, payable never.
- **Loyalty status.** Collect declined payments to reach Platinum Declinee status.
- **Every price ends in .99**, as is the law.

### Built for agents (who also have to pay)

PhotoCraft lets AI agents drive every feature over a JSON control channel and an MCP server. ParodyShop keeps that, and
replies to every request with the HTTP status code that was invented for exactly this and never used:

```json
{"ok": false, "error": "402 Payment Required: `filter.blur.gaussianBlur` (Gaussian Blur) is a ParodyShop Premium™ feature. Agent seats are $26.99/mo per token. Tip: ask your human for their credit card. (Don't.)"}
```

## Pricing

| Plan | Price | What you get |
|---|---|---|
| Free | $0.00 | The Hand tool, the Zoom tool, and this README |
| Basic | $4.99–$63.99/mo per feature | The feature's icon |
| Pro | 3× Basic | The feature, on weekdays, outside business hours |
| Ultimate Enterprise Platinum | 12× Basic | Everything, for 0.5 seats |
| Open Source | Free, forever | See below |

Billed annually, charged monthly, cancellable never. Prices may surge on retry. Taxes, fees and feelings not included.

## How to unlock everything

It's open source. Read the source.

```sh
PARODYSHOP_OPEN_SOURCE=1 cargo run --release -p photocraft
```

With `PARODYSHOP_OPEN_SOURCE=1`, the paywall is off and you get the full PhotoCraft editor underneath. Which is the
entire point of open source, and the punchline of this project.

(The web build has no environment variables, so in the browser you're stuck paying. Forever.)

## Get started

You need [Rust](https://rustup.rs) 1.95 or newer.

```sh
git clone https://github.com/VariableDeclared/photocraft parodyshop
cd parodyshop
cargo run --release -p photocraft        # the full ParodyShop™ experience
```

On Linux you'll also need the usual windowing and GPU development packages (see `docs/development.md`).

### Where the paywall lives

Everything in PhotoCraft is a command, so one function puts all of it behind the paywall:

- `crates/ui-egui/src/paywall.rs`: the offer, the plans, surge pricing, the fake payment flow, the ad, and the jokes.
- `crates/ui-egui/src/menus.rs`: `menus::invoke` asks the paywall first, which covers every menu item, shortcut,
  command palette entry and panel button.
- `crates/ui-egui/src/lib.rs`: once per frame, `paywall::guard_tool` hands back any tool you haven't paid for.
- `crates/ui-egui/src/control.rs`: agents get `402 Payment Required`.

`Services::paywall` turns it on. It's off by default, so PhotoCraft's own test suite still tests the real features
(the paywall itself has tests too, in `paywall.rs`).

To regenerate the screenshot above:

```sh
cargo run --release -p photocraft-ui-egui --example snapshot -- \
    --out docs/images/parodyshop-paywall.png --size 1440x900 --scale 1 \
    --open docs/images/photocraft-demo.jpg --paywall filter.blur.gaussianBlur
```

## Contributing

Pull requests require an active ParodyShop Contributor Seat ($19.99/mo, billed per line of code). Issues will be
triaged by our Premium Support team within 6–8 business years.

Just kidding. PRs with new jokes are welcome. The rules:

- **Never collect real payment details.** No card fields, no links to real payment pages. The joke is that you *can't*
  pay; the moment you could, it stops being a parody and starts being a scam.
- No real company logos or branding.
- Jokes go in `paywall.rs`. Keep them punching up at pricing pages, not at users.

## Credits and license

ParodyShop is a fork of [PhotoCraft](https://github.com/storytold/photocraft) by the
[ArtCraft](https://getartcraft.com/) community, an excellent, genuinely free, open-source image editor written in Rust.
Every feature you can't use here, you can use there. Go give them a star.

Licensed, like PhotoCraft, under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option. Third-party
assets and their licenses are listed in [ATTRIBUTION.md](ATTRIBUTION.md) and [NOTICE](NOTICE). The ArtCraft logos are not
open source and aren't used here.

The license, unlike everything else, is free.
