//! The rest of the ParodyShop Premium™ software-as-a-service experience, around the paywall
//! (`paywall`): a splash screen that phones home, a 9,000-page EULA you have to scroll to the end
//! of, a cookie banner with 400 partners, "We've updated our Terms" (Accept is the only answer),
//! "Your plan is changing" notices, usage-based billing per pixel hovered, half a seat, and an
//! Enterprise sales calendar booked until 2031.
//!
//! On whenever the paywall is (`Services::paywall`); every clock is egui's input time, so it runs
//! the same on the desktop and in the browser. Nothing here asks for or takes real payment details.

use std::ops::Range;

use egui::{Align2, RichText, vec2};

use crate::PhotocraftApp;

const SPLASH_SECS: f64 = 5.0;
const EULA_PAGES: usize = 9_000;
const LINES_PER_PAGE: usize = 12;
const PARTNERS: usize = 400;
/// The first "We've updated our Terms" this long after the EULA, then every [`TERMS_EVERY`] (s).
const TERMS_FIRST: f64 = 120.0;
const TERMS_EVERY: f64 = 600.0;
/// "Your plan is changing" this long after the EULA, then every [`PLAN_EVERY`] (s).
const PLAN_FIRST: f64 = 45.0;
const PLAN_EVERY: f64 = 300.0;
/// Usage-based billing, per pixel hovered.
const CENTS_PER_PIXEL: f64 = 0.01;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Onboarding {
    #[default]
    Splash,
    Eula,
    Done,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Cookies {
    #[default]
    Banner,
    Managing,
    Gone,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sales {
    Calendar,
    Booked,
}

#[derive(Default)]
pub struct Saas {
    pub(crate) enabled: bool,
    pub onboarding: Onboarding,
    pub cookies: Cookies,
    pub sales: Option<Sales>,
    /// Version of the Terms on screen, when "We've updated our Terms" is up.
    pub terms_open: Option<u32>,
    terms_version: u32,
    /// egui time the splash started and the EULA was accepted.
    splash_at: Option<f64>,
    accepted_at: Option<f64>,
    eula_at_end: bool,
    eula_jump: bool,
    next_terms: f64,
    next_plan: f64,
    plan_notices: usize,
    /// Pointer travel this session, in points: the usage meter.
    pub pixels_hovered: f64,
}

impl Saas {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, terms_version: 47, ..Default::default() }
    }

    /// Whether a SaaS modal (splash, EULA, Terms, partners, sales) is up.
    pub fn blocking(&self) -> bool {
        self.enabled && (self.onboarding != Onboarding::Done || self.terms_open.is_some() || self.cookies == Cookies::Managing || self.sales.is_some())
    }

    /// The usage bill so far, in cents.
    pub fn usage_cents(&self) -> u64 {
        (self.pixels_hovered.max(0.0) * CENTS_PER_PIXEL).min(1e12) as u64
    }

    /// Jump straight to after onboarding (tests, screenshots).
    pub fn skip_onboarding(&mut self, now: f64) {
        self.onboarding = Onboarding::Done;
        self.accept_eula(now);
    }

    fn accept_eula(&mut self, now: f64) {
        self.onboarding = Onboarding::Done;
        self.accepted_at = Some(now);
        self.next_terms = now + TERMS_FIRST;
        self.next_plan = now + PLAN_FIRST;
    }
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn pick<'a>(list: &[&'a str], n: u64) -> &'a str {
    list.get((n % list.len().max(1) as u64) as usize).copied().unwrap_or("")
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Once per frame, before anything is drawn: the clocks, the usage meter, and what's due.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.saas.enabled {
        return;
    }
    let (now, delta) = ctx.input(|i| (i.time, i.pointer.delta().length()));
    if delta.is_finite() {
        app.saas.pixels_hovered += f64::from(delta);
    }
    let Some(accepted) = app.saas.accepted_at else { return };
    // Engagement-based pricing: the longer you use it, the more it's worth to you.
    app.paywall.set_minutes_used((now - accepted) / 60.0);
    if app.saas.terms_open.is_none() && now >= app.saas.next_terms && !app.paywall.is_open() {
        app.saas.terms_version = app.saas.terms_version.saturating_add(1);
        app.saas.terms_open = Some(app.saas.terms_version);
        app.saas.next_terms = now + TERMS_EVERY;
    }
    if now >= app.saas.next_plan {
        let n = app.saas.plan_notices;
        app.saas.plan_notices = n.saturating_add(1);
        app.saas.next_plan = now + PLAN_EVERY;
        let line = pick(PLAN_CHANGES, n as u64).to_string();
        crate::notices::post(app, "Your plan is changing", vec![line, "No action is required. No action is possible.".into()], false, None);
    }
    // Keep the clocks running while nothing else asks for frames.
    ctx.request_repaint_after(std::time::Duration::from_secs(1));
}

/// Everything this module draws, over the app.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.saas.enabled {
        return;
    }
    let now = ctx.input(|i| i.time);
    match app.saas.onboarding {
        Onboarding::Splash => return splash(app, ctx, now),
        Onboarding::Eula => return eula(app, ctx, now),
        Onboarding::Done => {}
    }
    if let Some(v) = app.saas.terms_open {
        terms(app, ctx, v);
    } else if app.saas.sales.is_some() {
        sales(app, ctx);
    } else if app.saas.cookies == Cookies::Managing {
        partners(app, ctx);
    }
    if app.saas.cookies == Cookies::Banner && !app.saas.blocking() && !app.paywall.is_open() {
        cookie_banner(app, ctx);
    }
}

const SPLASH_STEPS: &[&str] = &[
    "Verifying license…",
    "Phoning home…",
    "Phoning home again (nobody answered)…",
    "Uploading your vibes to the cloud…",
    "Downloading 4 GB of mandatory updates to the updater…",
    "Checking you haven't paid for this somewhere else…",
    "Loading paywalls (1,213 of 1,213)…",
    "Almost done (this is a lie)…",
];

fn splash(app: &mut PhotocraftApp, ctx: &egui::Context, now: f64) {
    let since = *app.saas.splash_at.get_or_insert(now);
    let t = ((now - since) / SPLASH_SECS).clamp(0.0, 1.0);
    if t >= 1.0 {
        app.saas.onboarding = Onboarding::Eula;
        return;
    }
    ctx.request_repaint();
    // Rushes to 99%, then sits there.
    let progress = (t * 1.6).min(0.99) as f32;
    let step = ((t * SPLASH_STEPS.len() as f64) as usize).min(SPLASH_STEPS.len().saturating_sub(1));
    let mut skip = false;
    egui::Modal::new(egui::Id::new("parodyshop-splash")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("ParodyShop™").font(crate::theme::semibold(34.0)));
            ui.label(RichText::new("Premium Creative Clout Edition 2026.10 (Subscription Build)").small().weak());
            ui.add_space(16.0);
            ui.add(egui::ProgressBar::new(progress).show_percentage().desired_width(380.0));
            ui.add_space(6.0);
            ui.label(SPLASH_STEPS.get(step).copied().unwrap_or("…"));
            ui.add_space(10.0);
            skip = ui.small_button("Skip intro ($1.99)").clicked();
            ui.add_space(6.0);
        });
    });
    if skip {
        app.paywall.offer("Skipping the Intro", "saas.skipIntro");
    }
}

const CLAUSES: &[&str] = &[
    "The User grants ParodyShop a perpetual, irrevocable licence to the User's art, ideas, dreams and future grandchildren's art.",
    "ParodyShop may change the price at any time, including during this sentence.",
    "Pixels remain the property of ParodyShop and are leased to the User on a per-frame basis.",
    "The User agrees that \"unlimited\" means \"limited\" and that \"forever\" means \"until the next quarterly earnings call\".",
    "Cancellation requests must be submitted by fax, in triplicate, during a leap second.",
    "ParodyShop is not responsible for any features, as there are none.",
    "By reading this clause, the User has incurred a reading fee.",
    "The User consents to telemetry, including but not limited to clicks, keystrokes, sighs and facial expressions.",
    "This clause intentionally left blank. Blank clauses are billed at the standard rate.",
    "Nothing in this Agreement shall be construed as anything.",
    "The User waives the right to waive rights.",
    "ParodyShop may assign this Agreement to any party, including a raccoon.",
    "The free tier is provided \"as is\", where \"is\" is defined as \"isn't\".",
    "Any resemblance to a fair agreement is purely coincidental.",
];

fn eula_line(row: usize) -> String {
    let (page, line) = (row / LINES_PER_PAGE + 1, row % LINES_PER_PAGE);
    if line == 0 {
        return format!("— Page {} of {} —", thousands(page as u64), thousands(EULA_PAGES as u64));
    }
    let clause = pick(CLAUSES, hash(&row.to_string()));
    format!("§{page}.{line}  {clause}")
}

fn eula(app: &mut PhotocraftApp, ctx: &egui::Context, now: f64) {
    let rows = EULA_PAGES * LINES_PER_PAGE;
    let (mut agree, mut disagree) = (false, false);
    let jump = std::mem::take(&mut app.saas.eula_jump);
    let mut at_end = app.saas.eula_at_end;
    egui::Modal::new(egui::Id::new("parodyshop-eula")).show(ctx, |ui| {
        ui.set_width(620.0);
        ui.label(RichText::new("ParodyShop™ End User License Agreement").font(crate::theme::semibold(15.0)));
        ui.label(RichText::new(format!("{} pages · Estimated reading time: 41 days · Please read carefully, we didn't.", thousands(EULA_PAGES as u64))).small().weak());
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(6.0);
        let row_h = ui.text_style_height(&egui::TextStyle::Small) + 4.0;
        let mut area = egui::ScrollArea::vertical().id_salt("parodyshop-eula-text").max_height(320.0).auto_shrink([false, false]);
        if jump {
            area = area.vertical_scroll_offset(row_h * rows as f32);
        }
        let out = area.show_rows(ui, row_h, rows, |ui, range: Range<usize>| {
            for row in range {
                ui.label(RichText::new(eula_line(row)).small());
            }
        });
        let bottom = out.state.offset.y + out.inner_rect.height();
        at_end |= bottom >= out.content_size.y - row_h * 2.0;
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.small_button("Jump to the end (reading was optional anyway)").clicked() {
                app.saas.eula_jump = true;
            }
        });
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let label = if at_end { "I Agree" } else { "I Agree (scroll to the end first)" };
            agree = ui.add_enabled(at_end, egui::Button::new(label)).clicked();
            disagree = ui.button("I Disagree").clicked();
        });
    });
    app.saas.eula_at_end = at_end;
    if agree {
        app.saas.accept_eula(now);
        app.ui.status = "License accepted. Your soul has been backed up to the cloud.".into();
        app.ui.status_error = false;
    } else if disagree {
        app.paywall.offer("Disagreeing with the EULA", "saas.disagree");
    }
}

const TERMS_CHANGES: &[&str] = &[
    "We've replaced the word \"customer\" with \"revenue unit\" throughout.",
    "Section 4.2 (Your Rights) has been moved to Section 404 (Not Found).",
    "Arbitration will now take place in a hot-air balloon, at a location of our choosing.",
    "\"Free\" has been redefined. See Section 9,000 for the new definition.",
    "We may now contact you by carrier pigeon. Pigeon fees apply.",
    "Your data is now stored in the cloud. Specifically, a cumulonimbus.",
];

fn terms(app: &mut PhotocraftApp, ctx: &egui::Context, version: u32) {
    let mut accept = false;
    // No close button, no Esc, no clicking outside: Accept is the only answer.
    let _ = egui::Modal::new(egui::Id::new("parodyshop-terms")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.label(RichText::new("We've updated our Terms of Service").font(crate::theme::semibold(15.0)));
        ui.label(RichText::new(format!("Version {version}.0 · Effective retroactively")).small().weak());
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        ui.label("What's changed:");
        let n = u64::from(version);
        for i in 0..2 {
            ui.label(format!("• {}", pick(TERMS_CHANGES, n.wrapping_mul(3).wrapping_add(i))));
        }
        ui.label("• We've updated our Terms of Service.");
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            accept = ui.button("Accept").clicked();
        });
    });
    if accept {
        app.saas.terms_open = None;
    }
}

const PLAN_CHANGES: &[&str] = &[
    "Starting next month, the Free plan includes 0 tools (down from 2).",
    "We're raising prices so that we can keep prices low.",
    "Premium is now Premium Classic. The new Premium costs more and includes less.",
    "Your plan is being migrated to a new plan with the same features at a higher price.",
    "To serve you better, Zoom is moving to the Pro tier. You can still zoom out.",
    "Seats are now billed per half-seat. Your 0.5 seats have been rounded up to 1 (billed as 2).",
];

fn cookie_banner(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let (mut accept, mut manage, mut reject) = (false, false, false);
    egui::Area::new(egui::Id::new("parodyshop-cookies")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -40.0)).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(12).show(ui, |ui| {
            ui.set_max_width(760.0);
            ui.label(RichText::new("We value your privacy").strong());
            ui.label(format!(
                "We and our {PARTNERS} partners use cookies, and cookies about those cookies, to personalise your paywalls, measure how much you hover, \
                 and sell the results to the other {} partners. Rejecting them is a Premium feature.",
                PARTNERS - 1
            ));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                accept |= ui.button("Accept all").clicked();
                accept |= ui.button("Accept all (but in blue)").clicked();
                manage = ui.button(format!("Manage {PARTNERS} partners")).clicked();
                reject = ui.small_button("Reject all ($2.99)").clicked();
            });
        });
    });
    if accept {
        app.saas.cookies = Cookies::Gone;
    } else if manage {
        app.saas.cookies = Cookies::Managing;
    } else if reject {
        app.paywall.offer("Rejecting Cookies", "saas.rejectCookies");
    }
}

const PARTNER_FIRST: &[&str] = &["Data", "Pixel", "Cloud", "Synergy", "Hyper", "Track", "Quantum", "Blockchain", "Insight", "Funnel"];
const PARTNER_LAST: &[&str] =
    &["Brokers Ltd", "Analytics GmbH", "Harvest Inc.", "Partners & Partners", "Holdings", "Labs", "Dynamics", "Monetization AG", "Retargeting Co.", "Vibes LLC"];

fn partner(i: usize) -> String {
    let (f, l) = (PARTNER_FIRST.len(), PARTNER_LAST.len());
    let name = format!("{}{}", PARTNER_FIRST.get(i % f).copied().unwrap_or(""), PARTNER_LAST.get((i + i / f) % l).map(|s| format!(" {s}")).unwrap_or_default());
    if i >= f * l { format!("{name} {}", i / (f * l) + 1) } else { name }
}

fn partners(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut save = false;
    let modal = egui::Modal::new(egui::Id::new("parodyshop-partners")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.label(RichText::new(format!("Manage {PARTNERS} partners")).font(crate::theme::semibold(15.0)));
        ui.label(RichText::new("Every partner has a legitimate interest in you. Legitimate interests can't be switched off.").small().weak());
        ui.add_space(6.0);
        let row_h = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical().id_salt("parodyshop-partner-list").max_height(300.0).show_rows(ui, row_h, PARTNERS, |ui, range| {
            for i in range {
                ui.horizontal(|ui| {
                    let mut on = true;
                    ui.add_enabled(false, egui::Checkbox::new(&mut on, partner(i)));
                    ui.label(RichText::new("Legitimate interest").small().weak());
                });
            }
        });
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            save = ui.button("Save my choices").clicked();
        });
    });
    if save || modal.should_close() {
        app.saas.cookies = Cookies::Gone;
        app.ui.status = "Your choices have been saved and ignored.".into();
        app.ui.status_error = false;
    }
}

const MONTHS: &[&str] = &["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn sales(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(stage) = app.saas.sales else { return };
    let mut next = Some(stage);
    let modal = egui::Modal::new(egui::Id::new("parodyshop-sales")).show(ctx, |ui| {
        ui.set_width(440.0);
        match stage {
            Sales::Calendar => {
                ui.label(RichText::new("Enterprise: Contact Sales").font(crate::theme::semibold(15.0)));
                ui.label(RichText::new("Book a call with a ParodyShop Enterprise Solutions Synergy Partner. Pricing is available on request, and on request only.").small().weak());
                ui.add_space(6.0);
                egui::ScrollArea::vertical().id_salt("parodyshop-sales-calendar").max_height(260.0).show(ui, |ui| {
                    for year in 2027..=2031 {
                        for (m, month) in MONTHS.iter().enumerate() {
                            if year == 2031 && m == 2 {
                                if ui.button(format!("{month} {year}: Tue 3:00 AM, 1 slot available")).clicked() {
                                    next = Some(Sales::Booked);
                                }
                                return;
                            }
                            ui.label(RichText::new(format!("{month} {year}: Fully booked")).weak());
                        }
                    }
                });
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Never mind").clicked() {
                        next = None;
                    }
                });
            }
            Sales::Booked => {
                ui.label(RichText::new("You're booked!").font(crate::theme::semibold(15.0)));
                ui.add_space(6.0);
                ui.label("A sales representative will call you on Tuesday, 4 March 2031 at 3:00 AM to understand your needs, align on synergies and circle back.");
                ui.label(RichText::new("Please have your budget ready. They will reschedule.").small().weak());
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Can't wait").clicked() {
                        next = None;
                    }
                });
            }
        }
    });
    if modal.should_close() {
        next = None;
    }
    app.saas.sales = next;
}

/// The usage meter and seat count, at the end of the status bar.
pub fn status_meter(app: &PhotocraftApp, ui: &mut egui::Ui) {
    if !app.saas.enabled {
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    let cents = app.saas.usage_cents();
    let text = format!("Usage: {} px hovered · ${}.{:02} · Seats: 0.5/1", thousands(app.saas.pixels_hovered as u64), cents / 100, cents % 100);
    ui.label(RichText::new(text).small().color(t.text_faint)).on_hover_text(
        "Usage-based billing: $0.0001 per pixel your pointer hovers over. Hover less to save money.\n\
         Seats: you're using ParodyShop as half a user. Upgrade to a whole user with ParodyShop Premium™.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        let services = crate::Services { paywall: true, ..Default::default() };
        PhotocraftApp::new(photocraft_engine::Session::new(), services)
    }

    fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64) {
        let input = egui::RawInput { time: Some(time), ..Default::default() };
        ctx.run_ui(input, |ui| {
            tick(app, ui.ctx());
            show(app, ui.ctx());
        })
        .textures_delta
        .clear();
    }

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        ctx
    }

    #[test]
    fn launch_goes_splash_then_eula() {
        let (mut app, ctx) = (app(), ctx());
        assert!(app.saas.enabled);
        frame(&mut app, &ctx, 0.0);
        assert_eq!(app.saas.onboarding, Onboarding::Splash);
        frame(&mut app, &ctx, SPLASH_SECS + 0.1);
        assert_eq!(app.saas.onboarding, Onboarding::Eula);
        frame(&mut app, &ctx, SPLASH_SECS + 0.2);
        assert!(!app.saas.eula_at_end, "nobody reads 9,000 pages in a frame");
    }

    #[test]
    fn terms_and_plan_notices_come_on_schedule() {
        let (mut app, ctx) = (app(), ctx());
        app.saas.skip_onboarding(0.0);
        frame(&mut app, &ctx, PLAN_FIRST + 1.0);
        assert!(app.ui.notices.iter().any(|n| n.title == "Your plan is changing"));
        assert!(app.saas.terms_open.is_none());
        frame(&mut app, &ctx, TERMS_FIRST + 1.0);
        assert_eq!(app.saas.terms_open, Some(48));
    }

    #[test]
    fn using_it_longer_costs_more() {
        let (mut app, ctx) = (app(), ctx());
        app.saas.skip_onboarding(0.0);
        let before = app.paywall.price_cents("file.save");
        frame(&mut app, &ctx, 30.0 * 60.0);
        assert!(app.paywall.price_cents("file.save") > before);
    }

    #[test]
    fn every_modal_draws() {
        let (mut app, ctx) = (app(), ctx());
        app.saas.skip_onboarding(0.0);
        for (terms, sales, cookies) in [
            (Some(48), None, Cookies::Banner),
            (None, Some(Sales::Calendar), Cookies::Banner),
            (None, Some(Sales::Booked), Cookies::Banner),
            (None, None, Cookies::Managing),
        ] {
            app.saas.terms_open = terms;
            app.saas.sales = sales;
            app.saas.cookies = cookies;
            frame(&mut app, &ctx, 1.0);
        }
    }

    #[test]
    fn the_paperwork_is_well_formed() {
        assert_eq!(eula_line(0), "— Page 1 of 9,000 —");
        assert!(eula_line(EULA_PAGES * LINES_PER_PAGE - 1).starts_with("§9000.11"));
        assert_eq!(partner(0), "Data Brokers Ltd");
        assert!(partner(PARTNERS - 1).ends_with(" 4"));
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert!(!Saas::new(false).blocking());
    }

    #[test]
    fn off_by_default() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(!app.saas.enabled);
    }
}
