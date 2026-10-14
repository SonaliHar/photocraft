//! ParodyShop Premium™: the paywall in front of every feature.
//!
//! Every command from a menu, shortcut, palette or panel goes through [`intercept`] (called first
//! thing in `menus::invoke`), every tool switch through [`guard_tool`] (once per frame), and every
//! agent request through [`bill_agent`] (the control channel). Each one parks the feature behind a
//! subscription offer that can never be paid: no payment details are ever asked for, and every
//! "payment" is declined. Only the Hand and Zoom tools are free, so you can get a closer look at
//! what you're missing.
//!
//! The paywall is on when the app binary sets `Services::paywall` (the desktop and web apps do,
//! unless `PARODYSHOP_OPEN_SOURCE=1` is set: in open source, reading the code is how you unlock it).
//! `Services::default()` leaves it off, so the test suite still exercises the real features.

use std::collections::HashMap;

use egui::Key;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::widgets::{ButtonRole, DialogButton};

/// The environment variable that turns the paywall off.
pub const OPEN_SOURCE_ENV: &str = "PARODYSHOP_OPEN_SOURCE";

/// Tools on the free tier.
const FREE_TOOLS: &[Tool] = &[Tool::Hand, Tool::Zoom];

/// How long the payment "processes" before it is declined, and how long the ad runs (seconds).
const PROCESSING_SECS: f64 = 2.8;
const AD_SECS: f64 = 5.0;

/// Each retry of the same feature costs this much more (surge pricing).
const SURGE: f64 = 1.15;

/// Whether `PARODYSHOP_OPEN_SOURCE` asks for the paywall to be off.
pub fn open_source_mode(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
}

#[derive(Default)]
pub struct Paywall {
    /// Paywall on (set from `Services::paywall` when the app is built).
    pub(crate) enabled: bool,
    /// How many times each feature has been tried: drives surge pricing.
    attempts: HashMap<String, u32>,
    /// The offer on screen, if any.
    prompt: Option<Prompt>,
    /// The tool the user is allowed to hold (a free one).
    licensed_tool: Option<Tool>,
    /// Payments declined so far this session (shown as "loyalty").
    declined: u32,
}

struct Prompt {
    feature: String,
    key: String,
    stage: Stage,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Offer,
    Processing { since: f64 },
    Declined { reason: usize },
    Ad { since: f64 },
    AdDone,
    ConfirmCancel,
}

impl Paywall {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, ..Default::default() }
    }

    /// Whether an offer is on screen.
    pub fn is_open(&self) -> bool {
        self.prompt.is_some()
    }

    /// The feature the open offer is for.
    pub fn feature(&self) -> Option<&str> {
        self.prompt.as_ref().map(|p| p.feature.as_str())
    }

    /// The monthly price of `key` right now, in cents (surge pricing included).
    pub fn price_cents(&self, key: &str) -> u64 {
        let tries = self.attempts.get(key).copied().unwrap_or(0).min(60);
        let surged = base_cents(key) as f64 * SURGE.powi(tries.saturating_sub(1) as i32);
        // Every price ends in .99, as is the law.
        let dollars = (surged / 100.0).floor().clamp(0.0, 9_999_999.0) as u64;
        dollars.saturating_mul(100).saturating_add(99)
    }

    fn bill(&mut self, key: &str) {
        let n = self.attempts.entry(key.to_string()).or_insert(0);
        *n = n.saturating_add(1);
    }

    fn open(&mut self, feature: String, key: String) {
        self.bill(&key);
        // A second request while an offer is up keeps the one on screen (it already costs more).
        if self.prompt.is_none() {
            self.prompt = Some(Prompt { feature, key, stage: Stage::Offer });
        }
    }
}

/// FNV-1a: a stable hash, so a feature costs the same on every run and every machine.
fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// A feature's list price in cents, before surge pricing.
fn base_cents(key: &str) -> u64 {
    match key {
        // Regret is a luxury good.
        "edit.undo" | "edit.stepBackward" => 99_900,
        "edit.redo" | "edit.stepForward" => 149_900,
        "file.exit" => 4_999,
        _ => 499 + (hash(key) % 60) * 100,
    }
}

fn dollars(cents: u64) -> String {
    let (d, c) = (cents / 100, cents % 100);
    let mut s = d.to_string();
    let mut i = s.len();
    while i > 3 {
        i -= 3;
        s.insert(i, ',');
    }
    format!("${s}.{c:02}")
}

/// What the user calls a command: its menu label, else its engine label, else its id.
pub fn feature_label(id: &str) -> String {
    let menu = crate::menu_catalog::CATALOG.iter().find(|(_, label, _, cid)| *cid == id && *label != "---").map(|(_, label, ..)| *label);
    let label = menu.or_else(|| photocraft_engine::commands::find(id).map(|c| c.label)).unwrap_or(id);
    label.trim_end_matches('…').trim_end_matches("...").to_string()
}

/// The sales pitch for a feature.
fn pitch(key: &str) -> &'static str {
    const SPECIFIC: &[(&str, &str)] = &[
        ("edit.undo", "Regret is a Premium feature. Undo is billed per undo."),
        ("edit.redo", "Redo is billed per undo of the undo, plus a convenience fee."),
        ("file.exit", "Quitting requires ParodyShop Exit+. The window's close button is still free. For now."),
        ("file.new", "Blank canvases are a Premium feature. The emptiness is copyrighted."),
        ("file.open", "Opening files requires ParodyShop Open™, sold separately from ParodyShop Save™."),
        ("file.save", "Your work will be held safely in escrow until your payment clears."),
        ("file.saveAs", "Saving under a new name counts as a new seat."),
        ("file.print", "Printing is metered per pixel. Ink sold separately. Paper sold separately. Printer sold separately."),
        ("edit.copy", "Copying is $0.99 per copy. Bulk packs of 10 copies available for $14.99."),
        ("edit.paste", "Pasting requires a Paste Pass. Your clipboard has been placed in the cloud for safekeeping."),
        ("edit.preferences.general", "Preferences are a preference. Ours is that you pay."),
        ("help.about", "Knowing who made this app is an Enterprise-only feature."),
    ];
    const BY_PREFIX: &[(&str, &str)] = &[
        ("file.export", "Exports are free to start and $4.99 to finish. Your file is 99% exported."),
        ("file.", "File access is a cloud-first, file-last experience."),
        ("edit.", "Editing is included in ParodyShop Edit™, a separate subscription from ParodyShop™."),
        ("image.adjustments", "Adjustments now run on Generative AI™ in the Cloud™. Credits remaining: 0."),
        ("image.", "Images are licensed per pixel. You have used 0 of your 0 free pixels."),
        ("layer.", "The free tier includes 0 layers. Layers are like onions: they make you cry when you see the price."),
        ("type.", "Fonts are licensed per letter. You have 0 letters remaining. Vowels cost extra."),
        ("select.", "Selecting things is a choice. Choices are a paid feature."),
        ("filter.", "This filter is now AI-powered™. It costs 12 Generative Credits. You have 0 Generative Credits."),
        ("view.", "Looking at your own image requires a View License (one pair of eyes per seat)."),
        ("window.", "Panels are rented, not owned. Your lease has expired."),
        ("help.", "Help is available to Enterprise customers with a support contract. Response time: 6–8 business years."),
        ("tool.", "Tools are leased by the hour. Please return your tool to the toolbox in its original packaging."),
    ];
    const GENERIC: &[&str] = &[
        "This feature has been moved to a higher tier as part of our commitment to you.",
        "We've updated our pricing to better reflect the value we provide to our shareholders.",
        "This feature used to be free. We're as surprised as you are.",
        "Unlock this and 2 other features nobody uses with ParodyShop Premium™.",
    ];
    SPECIFIC
        .iter()
        .find(|(k, _)| *k == key)
        .or_else(|| BY_PREFIX.iter().find(|(p, _)| key.starts_with(p)))
        .map(|(_, s)| *s)
        .unwrap_or_else(|| GENERIC.get((hash(key) % GENERIC.len() as u64) as usize).copied().unwrap_or("Pay up."))
}

const PROCESSING: &[&str] = &[
    "Contacting Creative Clout™…",
    "Checking your credit score…",
    "Phoning home…",
    "Selling your browsing history to cover the transaction fee…",
    "Asking your bank if you're sure…",
    "Converting dollars to ParodyCoin™…",
];

const DECLINED: &[&str] = &[
    "Card declined: ParodyShop does not accept real money.",
    "Payment failed: your bank has been notified of your poor decisions.",
    "Declined: this card is registered to someone who can't draw.",
    "Error 402: Payment Required. Also, payment refused.",
    "Your payment was successful! Just kidding. It was declined.",
    "Declined: the price went up while we were processing it.",
];

/// The offer, or `None` for commands that stay free (none do, but the shell's own bookkeeping
/// that never reaches a person is let through).
pub fn intercept(app: &mut PhotocraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    if !app.paywall.enabled || app.automation_input {
        return None;
    }
    let feature = feature_label(id);
    app.paywall.open(feature.clone(), id.to_string());
    let cents = app.paywall.price_cents(id);
    app.ui.status = format!("🔒 {feature} is a ParodyShop Premium™ feature ({}/mo)", dollars(cents));
    app.ui.status_error = true;
    Some(Ok(json!({"paywalled": true, "feature": feature, "priceCents": cents})))
}

/// The error an agent (control channel, MCP) gets instead of a feature: there's no one to show the
/// offer to, so it gets the HTTP status that was invented for exactly this and never used.
pub fn bill_agent(app: &mut PhotocraftApp, id: &str) -> Option<String> {
    if !app.paywall.enabled {
        return None;
    }
    app.paywall.bill(id);
    let price = dollars(app.paywall.price_cents(id));
    Some(format!(
        "402 Payment Required: `{id}` ({}) is a ParodyShop Premium™ feature. Agent seats are {price}/mo per token. \
         Tip: ask your human for their credit card. (Don't.)",
        feature_label(id)
    ))
}

/// Once per frame: a tool the user hasn't paid for is handed back for the free one they had,
/// and they get the offer instead.
pub fn guard_tool(app: &mut PhotocraftApp) {
    if !app.paywall.enabled {
        return;
    }
    let tool = app.ui.tool;
    if FREE_TOOLS.contains(&tool) {
        app.paywall.licensed_tool = Some(tool);
        return;
    }
    let back = app.paywall.licensed_tool.unwrap_or(Tool::Hand);
    app.ui.tool = back;
    app.paywall.licensed_tool = Some(back);
    let feature = tool.label().to_string();
    let key = format!("tool.{feature}");
    app.paywall.open(feature.clone(), key.clone());
    app.ui.status = format!("🔒 The {feature} is a ParodyShop Premium™ feature ({}/mo)", dollars(app.paywall.price_cents(&key)));
    app.ui.status_error = true;
}

/// The offer, drawn over everything.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &app.paywall.prompt else { return };
    let now = ctx.input(|i| i.time);
    let (feature, key, stage) = (p.feature.clone(), p.key.clone(), p.stage);
    let price = app.paywall.price_cents(&key);
    let declined = app.paywall.declined;
    let mut next: Option<Option<Stage>> = None;
    let mut close = false;

    // Timed stages move on by themselves.
    match stage {
        Stage::Processing { since } if now - since >= PROCESSING_SECS => {
            let reason = (hash(&key).wrapping_add(u64::from(declined)) % DECLINED.len() as u64) as usize;
            app.paywall.declined = app.paywall.declined.saturating_add(1);
            next = Some(Some(Stage::Declined { reason }));
        }
        Stage::Ad { since } if now - since >= AD_SECS => next = Some(Some(Stage::AdDone)),
        Stage::Processing { .. } | Stage::Ad { .. } => ctx.request_repaint(),
        _ => {}
    }

    let modal = egui::Modal::new(egui::Id::new("parodyshop-paywall")).show(ctx, |ui| {
        ui.set_max_width(460.0);
        let title = match stage {
            Stage::Offer => format!("🔒 {feature} is a Premium feature"),
            Stage::Processing { .. } => "Processing payment…".to_string(),
            Stage::Declined { .. } => "Payment declined".to_string(),
            Stage::Ad { .. } | Stage::AdDone => "A word from our sponsor (us)".to_string(),
            Stage::ConfirmCancel => "Are you sure?".to_string(),
        };
        ui.label(egui::RichText::new(title).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        let mut buttons: Vec<(ButtonRole, &str, Option<Stage>, bool)> = Vec::new();
        match stage {
            Stage::Offer => {
                ui.label(pitch(&key));
                ui.add_space(10.0);
                plans(ui, price);
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Billed annually, charged monthly, cancellable never. Prices may surge on retry. Taxes, fees and feelings not included.").small().weak());
                if declined > 0 {
                    ui.label(egui::RichText::new(format!("Loyalty status: {declined} declined payment{} (Platinum Declinee)", if declined == 1 { "" } else { "s" })).small().weak());
                }
                buttons = vec![
                    (ButtonRole::Default, "Subscribe", Some(Stage::Processing { since: now }), false),
                    (ButtonRole::Alternate, "Watch an ad", Some(Stage::Ad { since: now }), false),
                    (ButtonRole::Cancel, "No thanks", Some(Stage::ConfirmCancel), false),
                ];
            }
            Stage::Processing { since } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let step = (((now - since) / 0.6).max(0.0) as usize).min(PROCESSING.len().saturating_sub(1));
                    ui.label(PROCESSING.get(step).copied().unwrap_or("…"));
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Please don't close this window. Or do. It doesn't matter.").small().weak());
            }
            Stage::Declined { reason } => {
                ui.label(DECLINED.get(reason).copied().unwrap_or("Declined."));
                ui.add_space(6.0);
                ui.label(egui::RichText::new(format!("New price after your failed attempt: {}/mo", dollars(app.paywall.price_cents(&key)))).small().weak());
                buttons = vec![(ButtonRole::Default, "Try another card", None, true), (ButtonRole::Cancel, "Give up", None, false)];
            }
            Stage::Ad { since } => {
                ad(ui);
                ui.add_space(8.0);
                let left = (AD_SECS - (now - since)).ceil().max(1.0);
                ui.label(egui::RichText::new(format!("Your ad will end in {left:.0}… (Skip Ad available with ParodyShop Premium™)")).small().weak());
            }
            Stage::AdDone => {
                ui.label("Thank you for watching! Your ad revenue of $0.0003 has been applied to your account.");
                ui.add_space(4.0);
                ui.label(format!("Remaining balance for {feature}: {}/mo.", dollars(price.saturating_sub(1))));
                buttons = vec![(ButtonRole::Default, "Watch another ad", Some(Stage::Ad { since: now }), false), (ButtonRole::Cancel, "Close", None, false)];
            }
            Stage::ConfirmCancel => {
                ui.label("Real artists pay. Are you sure you want to keep your money?");
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Declining a subscription you don't have incurs an early termination fee of $84.99, payable never.").small().weak());
                buttons = vec![
                    (ButtonRole::Default, "Take me back to the offer", Some(Stage::Offer), false),
                    (ButtonRole::Cancel, "Yes, I'm sure", None, false),
                ];
            }
        }
        if !buttons.is_empty() {
            ui.add_space(12.0);
            let row: Vec<DialogButton> = buttons.iter().map(|b| DialogButton::new(b.0, b.1, 96.0)).collect();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if let Some(role) = crate::widgets::dialog_buttons(ui, &row)
                    && let Some(b) = buttons.iter().find(|b| b.0 == role)
                {
                    match (b.2, b.3) {
                        // Another card: back to the (now pricier) offer.
                        (_, true) => {
                            app.paywall.bill(&key);
                            next = Some(Some(Stage::Offer));
                        }
                        (Some(s), false) => next = Some(Some(s)),
                        (None, false) => close = true,
                    }
                }
            });
        }
    });
    // Esc and clicking outside close the offer, except mid-payment and mid-ad (you will watch it).
    let busy = matches!(stage, Stage::Processing { .. } | Stage::Ad { .. });
    if modal.should_close() && !busy {
        close = true;
    }
    if next.is_none() && !busy && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) && stage == Stage::Offer {
        next = Some(Some(Stage::Processing { since: now }));
    }
    if close {
        app.paywall.prompt = None;
    } else if let Some(Some(s)) = next
        && let Some(p) = app.paywall.prompt.as_mut()
    {
        p.stage = s;
    }
}

/// The three plans, the most expensive marked as the most popular.
fn plans(ui: &mut egui::Ui, cents: u64) {
    let rows = [
        ("Basic", cents, "Includes this feature's icon. Not the feature."),
        ("Pro", cents.saturating_mul(3), "Includes the feature, on weekdays, outside business hours."),
        ("Ultimate Enterprise Platinum", cents.saturating_mul(12), "★ Most popular ★ Everything, for 0.5 seats."),
    ];
    egui::Grid::new("parodyshop-plans").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
        for (name, price, blurb) in rows {
            ui.label(egui::RichText::new(name).strong());
            ui.label(format!("{}/mo", dollars(price)));
            ui.label(egui::RichText::new(blurb).small());
            ui.end_row();
        }
    });
}

fn ad(ui: &mut egui::Ui) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(400.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("Tired of paywalls?").font(crate::theme::semibold(18.0)));
            ui.label("Upgrade to ParodyShop Premium™ and get even more paywalls, in 4K!");
            ui.label(egui::RichText::new("ParodyShop: Imagine What You Could Do If You Paid.").italics().weak());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(enabled: bool) -> PhotocraftApp {
        let services = crate::Services { paywall: enabled, ..Default::default() };
        PhotocraftApp::new(photocraft_engine::Session::new(), services)
    }

    #[test]
    fn every_menu_command_asks_for_money() {
        let mut app = app(true);
        let ctx = egui::Context::default();
        let r = crate::menus::invoke(&mut app, &ctx, "file.new", json!({"width": 8, "height": 8})).unwrap();
        assert_eq!(r["paywalled"], true);
        assert!(app.session.documents().is_empty(), "the feature must not run");
        assert_eq!(app.paywall.feature(), Some("New"));
    }

    #[test]
    fn the_paywall_is_off_by_default_so_the_features_still_work() {
        let mut app = app(false);
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "file.new", json!({"width": 8, "height": 8})).unwrap();
        assert_eq!(app.session.documents().len(), 1);
        assert!(!app.paywall.is_open());
    }

    #[test]
    fn retrying_surges_the_price() {
        let mut app = app(true);
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "layer.new.layer", json!({})).unwrap();
        let first = app.paywall.price_cents("layer.new.layer");
        crate::menus::invoke(&mut app, &ctx, "layer.new.layer", json!({})).unwrap();
        assert!(app.paywall.price_cents("layer.new.layer") > first);
        assert_eq!(first % 100, 99, "prices end in .99");
    }

    #[test]
    fn agents_get_402() {
        let mut app = app(true);
        let e = bill_agent(&mut app, "filter.blur.gaussianBlur").unwrap();
        assert!(e.starts_with("402 Payment Required"), "{e}");
        assert!(bill_agent(&mut self::app(false), "filter.blur.gaussianBlur").is_none());
    }

    #[test]
    fn paid_tools_are_handed_back() {
        let mut app = app(true);
        assert_eq!(app.ui.tool, Tool::Hand, "the free tier starts on the Hand tool");
        app.ui.tool = Tool::Zoom;
        guard_tool(&mut app);
        assert_eq!(app.ui.tool, Tool::Zoom, "Zoom is free");
        app.ui.tool = Tool::Brush;
        guard_tool(&mut app);
        assert_eq!(app.ui.tool, Tool::Zoom);
        assert_eq!(app.paywall.feature(), Some("Brush Tool"));
    }

    #[test]
    fn prices_are_stable_and_formatted() {
        assert_eq!(base_cents("file.save"), base_cents("file.save"));
        assert_eq!(dollars(149_999), "$1,499.99");
        assert_eq!(dollars(99), "$0.99");
        assert!(Paywall::new(true).price_cents("edit.redo") > Paywall::new(true).price_cents("edit.undo"));
    }

    #[test]
    fn huge_retry_counts_dont_overflow() {
        let mut p = Paywall::new(true);
        p.attempts.insert("edit.undo".into(), u32::MAX);
        assert!(p.price_cents("edit.undo") > 0);
    }

    #[test]
    fn open_source_mode_reads_the_env_value() {
        assert!(open_source_mode(Some("1")));
        assert!(open_source_mode(Some("true")));
        assert!(!open_source_mode(Some("0")));
        assert!(!open_source_mode(None));
    }

    #[test]
    fn the_offer_draws_in_every_stage() {
        let mut app = app(true);
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        crate::menus::invoke(&mut app, &ctx, "edit.undo", json!({})).unwrap();
        for stage in [Stage::Offer, Stage::Processing { since: 0.0 }, Stage::Declined { reason: 99 }, Stage::Ad { since: 0.0 }, Stage::AdDone, Stage::ConfirmCancel] {
            if let Some(p) = app.paywall.prompt.as_mut() {
                p.stage = stage;
            }
            ctx.run_ui(Default::default(), |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        }
    }
}
