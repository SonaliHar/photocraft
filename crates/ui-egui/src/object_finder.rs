//! Object Selection › Object Finder (#4087): with the Object Selection tool active, resting the
//! pointer on the canvas highlights the object a click there would select, as Photoshop's Object
//! Finder does. A click (without dragging) commits that object (`select.object` with `"point"`;
//! ⇧ adds, ⌥ subtracts, ⇧⌥ intersects); dragging keeps the rectangle mode.
//!
//! The checked engine prepares a private candidate after the normal debounce.
//! Native workers poll a real cancellation token; web work remains inline. Only
//! a current snapshot/view plus matching engine cache witness can publish a result.
//! Successful misses and failures are distinct bounded memo entries. No document
//! or history mutation, source access, or render qualification is granted here.

use std::sync::Arc;

use egui::{Color32, TextureOptions, pos2};
use photocraft_algo::selection::Region;
use photocraft_engine::object_finder_controlled::{self as query, FinderLimits};
use photocraft_raster::Interrupt;
#[path="object_finder_worker.rs"]
mod worker;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// Seconds the pointer must rest before an object is looked up.
pub const DEBOUNCE: f64 = 0.12;
/// Longest side of the overlay texture (the overlay is a preview, not the selection).
const TEX_MAX: usize = 768;

/// Runtime-only source/view witness and derived preview, never serialized authority.
pub(crate) struct Finder {
    frame:worker::Frame,target:Option<query::FinderTarget>,
    at:[i32;2],since:f64,job:Option<worker::Job>,
    misses:Vec<[i32;2]>,failures:Vec<[i32;2]>,blocked:bool,
    shown:Option<Arc<Region>>,texture:Option<(usize,egui::TextureHandle)>,
}

/// What [`update`] decided.
#[derive(Clone, Debug, Default)]
pub(crate) struct Update {
    /// The object to highlight.
    pub shown: Option<Arc<Region>>,
    /// A lookup is pending or running: repaint soon to pick it up.
    pub busy: bool,
}

/// Whether the preview applies now (`pressed`: a pointer button is down).
fn active(app:&PhotocraftApp,pressed:bool)->bool {
    app.ui.tool==Tool::ObjectSelection && app.ui.tool_options.object_finder && !pressed && app.drag.is_none()
        && app.ui.transform.is_none() && !app.session.has_jobs() && app.session.active().is_some_and(|state|
            state.revision!=u64::MAX && state.floating.is_none() && state.coalesce.is_none()
            && state.doc.xcf_state.as_ref().is_none_or(|state|state.floating.is_none()))
}
fn nearby(points:&[[i32;2]],at:[i32;2])->bool {
    points.iter().any(|p|(i64::from(p[0])-i64::from(at[0])).abs()<=2 && (i64::from(p[1])-i64::from(at[1])).abs()<=2)
}
fn memo(points:&mut Vec<[i32;2]>,at:[i32;2])->Result<(),String> {
    if points.len()>=64 {points.remove(0);}
    if points.len()>=points.capacity(){return Err(tl!("Object Finder preview storage is unavailable.").to_owned());}points.push(at);Ok(())
}
fn error(app:&mut PhotocraftApp,finder:&mut Finder,at:[i32;2]) {
    if memo(&mut finder.failures,at).is_err(){finder.blocked=true;}
    finder.shown=None;finder.texture=None;
    app.ui.status=tl!("Object Finder could not prepare a preview here. Move the pointer to retry.").to_owned();app.ui.status_error=true;
}
/// Advance only view state. Expensive cache comparison/preparation occurs on
/// context/pointer changes or an explicit debounced attempt, never every frame.
pub(crate) fn update(app:&mut PhotocraftApp,at:Option<[f64;2]>,pressed:bool,now:f64)->Update {
    let at=at.filter(|p|p.iter().all(|v|v.is_finite() && v.abs()<=1e7));
    let (Some(at),true)=(at,active(app,pressed) && now.is_finite()) else {clear(app);return Update::default();};
    let px=[at[0].floor() as i32,at[1].floor() as i32];
    let changed=app.object_finder.as_ref().is_none_or(|finder|!finder.frame.current(app) || finder.target.as_ref().is_some_and(|target|!query::is_current(&app.session,target)));
    let old=app.object_finder.take();
    let mut finder=if changed {
        drop(old); // Cancels the old private candidate before source recapture.
        let frame=match worker::Frame::capture(app){Ok(frame)=>frame,Err(_)=>{app.ui.status=tl!("Object Finder preview storage is unavailable.").to_owned();app.ui.status_error=true;return Update::default();}};
        let mut misses=Vec::new();let mut failures=Vec::new();
        if misses.try_reserve_exact(64).is_err() || failures.try_reserve_exact(64).is_err(){app.ui.status=tl!("Object Finder preview storage is unavailable.").to_owned();app.ui.status_error=true;return Update::default();}
        Finder {frame,target:None,at:px,since:now,job:None,misses,failures,blocked:false,shown:None,texture:None}
    } else {let Some(finder)=old else {return Update::default();};finder};
    let moved=finder.at!=px;
    if moved {finder.at=px;finder.since=now;finder.job=None;}
    if let Some(outcome)=finder.job.as_ref().and_then(worker::Job::poll) {
        let job_at=finder.job.as_ref().map(|job|job.at);finder.job=None;
        if job_at==Some(px) && finder.frame.current(app) && active(app,pressed) {
            match outcome {
                worker::Outcome::Ready(prepared)=>if prepared.region().is_some_and(|region|region_shape(region).is_err()){error(app,&mut finder,px);}else{match query::publish(&mut app.session,prepared,&Interrupt::NONE) {
                    Ok(region)=>{
                        if let Some(region)=region {finder.shown=Some(region);finder.texture=None;}
                        else if memo(&mut finder.misses,px).is_err(){error(app,&mut finder,px);}
                        // Bind our own publication so it does not invalidate a
                        // successful miss on the next frame. Only later external
                        // cache changes should clear that memo.
                        match query::capture(&app.session,app.ui.tool_options.sample_all_layers,[at[0] as f32,at[1] as f32],FinderLimits::default(),&Interrupt::NONE) {
                            Ok(target)=>finder.target=Some(target),Err(_)=>error(app,&mut finder,px),
                        }
                    },
                    Err(_)=>error(app,&mut finder,px),
                }},
                worker::Outcome::Cancelled=>{},worker::Outcome::Failed=>error(app,&mut finder,px),
            }
        }
    }
    // Pointer-only cache identity is cheap and also notices external publication
    // while the pointer rests. A running candidate keeps its original witness;
    // publish owns the concurrent-cache refusal rather than hiding that race.
    let cache_changed=finder.job.is_none() && finder.target.as_ref().is_some_and(|target|!query::cache_is_current(&app.session,target));
    if cache_changed {finder.target=None;finder.misses.clear();finder.failures.clear();finder.blocked=false;finder.shown=None;finder.texture=None;}
    let inside=|region:&Region|region.at(px[0],px[1])>=0.5;
    let missed=nearby(&finder.misses,px);let failed=finder.blocked || nearby(&finder.failures,px);
    if !finder.shown.as_deref().is_some_and(inside) {finder.shown=None;}
    let in_bounds=app.session.active().is_some_and(|state|state.doc.bounds().contains(px[0],px[1]));
    // Lookup on a changed pixel or debounced attempt can reuse the checked
    // engine cache. Capturing carries no source locator or payload authority.
    if finder.shown.is_none() && finder.job.is_none() && !missed && !failed && in_bounds
        && (moved || changed || cache_changed || now-finder.since>=DEBOUNCE) {
        match query::capture(&app.session,app.ui.tool_options.sample_all_layers,[at[0] as f32,at[1] as f32],FinderLimits::default(),&Interrupt::NONE) {
            Ok(target)=>{finder.target=Some(target.clone());match query::lookup(&app.session,&target,[at[0] as f32,at[1] as f32],&Interrupt::NONE) {
                Ok(Some(region))=>{if region_shape(&region).is_ok(){finder.shown=Some(region);}else{error(app,&mut finder,px);}},
                Ok(None) if now-finder.since>=DEBOUNCE=>match worker::spawn(target,px){Ok(job)=>finder.job=Some(job),Err(_)=>error(app,&mut finder,px)},
                Ok(None)=>{},Err(_)=>error(app,&mut finder,px),
            }},
            Err(_)=>error(app,&mut finder,px),
        }
    }
    if !finder.frame.current(app) || finder.target.as_ref().is_some_and(|target|!query::is_current(&app.session,target)) || !active(app,pressed) {return Update::default();}
    let busy=finder.job.is_some() || (finder.shown.is_none() && !nearby(&finder.misses,px) && !finder.blocked && !nearby(&finder.failures,px) && in_bounds && now-finder.since<DEBOUNCE);
    let result=Update {shown:finder.shown.clone(),busy};app.object_finder=Some(finder);result
}

/// Clears the preview (a tool switch or an edit made elsewhere is caught by [`update`] too).
pub(crate) fn clear(app: &mut PhotocraftApp) {
    app.object_finder = None;
}

/// The overlay colour: the theme accent, translucent (Photoshop tints the object it finds).
fn overlay(accent: Color32, cov: f32, edge: bool) -> Color32 {
    let a = if edge { 0.95 } else { 0.42 * cov.clamp(0.0, 1.0) };
    Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), (a * 255.0).round() as u8)
}

/// The overlay image of `r`: translucent fill with an opaque one-texel outline, at most
/// [`TEX_MAX`] texels on its longest side.
fn region_shape(region:&Region)->Result<(usize,usize),String> {
    let w=i64::from(region.bbox.x1)-i64::from(region.bbox.x0);let h=i64::from(region.bbox.y1)-i64::from(region.bbox.y0);
    let failure=||tl!("Object Finder returned an invalid preview.").to_owned();
    if w<=0 || h<=0 || w>i64::from(i32::MAX) || h>i64::from(i32::MAX){return Err(failure());}
    let (w,h)=(usize::try_from(w).map_err(|_|failure())?,usize::try_from(h).map_err(|_|failure())?);
    let count=w.checked_mul(h).ok_or_else(failure)?;
    if count!=region.mask.len() || u64::try_from(count).ok().is_none_or(|count|count>FinderLimits::default().resident_bytes){return Err(failure());}
    Ok((w,h))
}
pub(crate) fn overlay_image(region:&Region,accent:Color32)->Result<egui::ColorImage,String> {
    let (w,h)=region_shape(region)?;let step=w.max(h).div_ceil(TEX_MAX).max(1);let (tw,th)=(w.div_ceil(step),h.div_ceil(step));
    let failure=||tl!("Object Finder preview storage is unavailable.").to_owned();
    let count=tw.checked_mul(th).filter(|count|*count<=TEX_MAX*TEX_MAX).ok_or_else(failure)?;
    let cov=|tx:usize,ty:usize|->Result<f32,String> {
        let x=tx.checked_mul(step).ok_or_else(failure)?;let y=ty.checked_mul(step).ok_or_else(failure)?;
        if x>=w || y>=h {return Ok(0.);}
        let index=y.checked_mul(w).and_then(|i|i.checked_add(x)).ok_or_else(failure)?;
        Ok(f32::from(*region.mask.get(index).ok_or_else(failure)?)/255.)
    };
    let mut px=Vec::new();px.try_reserve_exact(count).map_err(|_|failure())?;
    for ty in 0..th {
        Interrupt::NONE.check().map_err(|_|failure())?;
        for tx in 0..tw {
            let c=cov(tx,ty)?;let on=c>=0.5;
            let edge=on && (tx==0 || ty==0 || tx+1==tw || ty+1==th || cov(tx-1,ty)?<0.5 || cov(tx+1,ty)?<0.5 || cov(tx,ty-1)?<0.5 || cov(tx,ty+1)?<0.5);
            px.push(if c<=0. {Color32::TRANSPARENT}else{overlay(accent,c,edge)});
        }
    }
    Ok(egui::ColorImage::new([tw,th],px))
}

/// Updates the preview for this frame and draws the highlight on the active document's canvas.
pub(crate) fn draw(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, idx: usize, hover: Option<egui::Pos2>) {
    if app.session.active_index() != Some(idx) {
        return;
    }
    let ctx = painter.ctx().clone();
    let (pressed, now) = ctx.input(|i| (i.pointer.any_down(), i.time));
    let up = update(app, hover.map(|p| xf.to_doc(p)), pressed, now);
    if up.busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(40));
    }
    let Some(r) = up.shown else { return };
    let Some(f) = app.object_finder.as_mut() else { return };
    let key = Arc::as_ptr(&r) as usize;
    if f.texture.as_ref().is_none_or(|(k, _)| *k != key) {
        let accent = crate::theme::Tokens::get(&ctx).accent;
        let image=match overlay_image(&r,accent){Ok(image)=>image,Err(error)=>{f.shown=None;f.texture=None;app.ui.status=error;app.ui.status_error=true;return;}};
        let tex = ctx.load_texture("object-finder", image, TextureOptions::LINEAR);
        f.texture = Some((key, tex));
    }
    let Some((_, tex)) = &f.texture else { return };
    let b = r.bbox;
    let mut mesh = egui::Mesh::with_texture(tex.id());
    let corners = [(b.x0, b.y0, 0.0, 0.0), (b.x1, b.y0, 1.0, 0.0), (b.x1, b.y1, 1.0, 1.0), (b.x0, b.y1, 0.0, 1.0)];
    for (x, y, u, v) in corners {
        mesh.vertices.push(egui::epaint::Vertex { pos: xf.to_screen(x as f32, y as f32), uv: pos2(u, v), color: Color32::WHITE });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A 200 × 160 document: a red disc (centre 100, 80, radius 45) on blue.
    pub(crate) fn disc_app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 160})).unwrap();
        app.session
            .edit("paint", |doc, a| {
                let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
                s.fill_rect(photocraft_geom::Rect::new(0, 0, 200, 160), &[0.2, 0.4, 0.8, 1.0]);
                for y in 0..160 {
                    for x in 0..200 {
                        let (dx, dy) = (x as f32 + 0.5 - 100.0, y as f32 + 0.5 - 80.0);
                        if dx * dx + dy * dy <= 45.0 * 45.0 {
                            s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[0.9, 0.2, 0.1, 1.0]);
                        }
                    }
                }
                Ok(())
            })
            .unwrap();
        app.ui.tool = Tool::ObjectSelection;
        app
    }

    /// Hovers at `at` until the lookup settles (time advances past the debounce).
    pub(crate) fn hover(app: &mut PhotocraftApp, at: [f64; 2]) -> Update {
        let mut t = 100.0;
        loop {
            let u = update(app, Some(at), false, t);
            if !u.busy || t > 130.0 {
                return u;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            t += 0.05;
        }
    }

    #[test]
    fn hovering_an_object_highlights_it_after_a_rest() {
        let mut app = disc_app();
        // Not before the pointer has rested.
        let u = update(&mut app, Some([100.0, 80.0]), false, 50.0);
        assert!(u.shown.is_none() && u.busy);
        let u = hover(&mut app, [100.0, 80.0]);
        let r = u.shown.expect("the disc is highlighted");
        assert!(r.at(100, 80) >= 0.5 && r.at(60, 80) >= 0.5 && r.at(5, 5) < 0.5);
        // Elsewhere on the same object: served from the cache, no new lookup.
        let u = update(&mut app, Some([120.0, 90.0]), false, 200.0);
        assert!(u.shown.is_some_and(|s| Arc::ptr_eq(&s, &r)) && !u.busy);
        assert_eq!(app.session.object_finder.len(), 1);
        // The background highlights nothing.
        assert!(hover(&mut app, [4.0, 4.0]).shown.is_none());
    }

    #[test]
    fn preview_clears_on_leave_tool_switch_button_option_and_edits() {
        let mut app = disc_app();
        assert!(hover(&mut app, [100.0, 80.0]).shown.is_some());
        assert!(update(&mut app, None, false, 300.0).shown.is_none());
        assert!(app.object_finder.is_none());
        assert!(hover(&mut app, [100.0, 80.0]).shown.is_some());
        assert!(update(&mut app, Some([100.0, 80.0]), true, 300.0).shown.is_none());
        app.ui.tool = Tool::Lasso;
        assert!(update(&mut app, Some([100.0, 80.0]), false, 300.0).shown.is_none());
        app.ui.tool = Tool::ObjectSelection;
        app.ui.tool_options.object_finder = false;
        assert!(update(&mut app, Some([100.0, 80.0]), false, 300.0).shown.is_none());
        app.ui.tool_options.object_finder = true;
        assert!(hover(&mut app, [100.0, 80.0]).shown.is_some());
        // A pixel edit: the cached object is dropped and the preview starts over.
        app.session.execute("paint.pencil", json!({"points": [[5, 5], [30, 5]], "size": 4, "color": "#000000"})).unwrap();
        let u = update(&mut app, Some([100.0, 80.0]), false, 400.0);
        assert!(u.shown.is_none() && u.busy);
    }

    #[test]
    fn click_commits_the_highlighted_object_with_the_modifiers() {
        use crate::canvas::{ToolEvent, tool_event};
        let mut app = disc_app();
        assert!(hover(&mut app, [100.0, 80.0]).shown.is_some());
        let click = |app: &mut PhotocraftApp, m: egui::Modifiers| {
            tool_event(app, ToolEvent::Down { x: 100.0, y: 80.0, pressure: 1.0 }, m);
            tool_event(app, ToolEvent::Up { x: 100.0, y: 80.0 }, m);
        };
        let cov = |app: &PhotocraftApp, x: i32, y: i32| app.session.active().unwrap().doc.selection.as_ref().map_or(0.0, |s| s.sample_channel(x, y, 0));
        click(&mut app, egui::Modifiers::NONE);
        assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("select.object"));
        assert!(cov(&app, 100, 80) >= 0.5 && cov(&app, 60, 80) >= 0.5 && cov(&app, 5, 5) < 0.5);
        app.run("select.all", json!({})).unwrap();
        click(&mut app, egui::Modifiers::ALT);
        assert!(cov(&app, 100, 80) < 0.5 && cov(&app, 5, 5) >= 0.5);
        click(&mut app, egui::Modifiers::SHIFT);
        assert!(cov(&app, 100, 80) >= 0.5 && cov(&app, 5, 5) >= 0.5);
        click(&mut app, egui::Modifiers { alt: true, shift: true, ..Default::default() });
        assert!(cov(&app, 100, 80) >= 0.5 && cov(&app, 5, 5) < 0.5);
        // Off: a click keeps the Quick Selection fallback.
        app.ui.tool_options.object_finder = false;
        click(&mut app, egui::Modifiers::NONE);
        assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("select.quick"));
    }

    #[test]
    fn overlay_image_tints_the_object_and_outlines_it() {
        let r = Region { bbox: photocraft_geom::Rect::new(0, 0, 4, 4), mask: vec![255; 16] };
        let img = overlay_image(&r, Color32::from_rgb(0, 120, 255)).unwrap();
        assert_eq!(img.size, [4, 4]);
        assert!(img.pixels[0].a() > 200, "edge");
        assert!(img.pixels[5].a() > 0 && img.pixels[5].a() < 200, "fill");
        let big = Region { bbox: photocraft_geom::Rect::new(0, 0, 3000, 10), mask: vec![255; 30_000] };
        assert!(overlay_image(&big, Color32::WHITE).unwrap().size[0] <= TEX_MAX);
    }
    #[test]
    fn failed_worker_is_not_a_successful_miss_and_same_pixel_is_not_retried_each_frame() {
        let mut app=disc_app();update(&mut app,Some([100.,80.]),false,1.);
        let (job,_)=worker::test_job([100,80],true);app.object_finder.as_mut().unwrap().job=Some(job);
        let before=app.session.active().unwrap().doc.clone();let history=app.session.active().unwrap().history.past_len();
        let result=update(&mut app,Some([100.,80.]),false,1.2);assert!(result.shown.is_none());assert!(!result.busy);assert!(app.ui.status_error);
        let finder=app.object_finder.as_ref().unwrap();assert!(finder.misses.is_empty());assert_eq!(finder.failures,vec![[100,80]]);
        update(&mut app,Some([100.,80.]),false,2.);assert!(app.object_finder.as_ref().unwrap().job.is_none());
        assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.active().unwrap().history.past_len(),history);
    }
    #[test]
    fn view_change_or_button_press_cancels_pending_candidate_without_overlay_publication() {
        use std::sync::atomic::Ordering;
        let mut app=disc_app();update(&mut app,Some([100.,80.]),false,1.);
        let (job,cancel)=worker::test_job([100,80],false);app.object_finder.as_mut().unwrap().job=Some(job);
        app.session.active_mut().unwrap().channel_view.quick_mask_hidden=true;
        let result=update(&mut app,Some([100.,80.]),false,1.1);assert!(cancel.load(Ordering::Acquire));assert!(result.shown.is_none());
        let (job,cancel)=worker::test_job([100,80],false);app.object_finder.as_mut().unwrap().job=Some(job);
        update(&mut app,Some([100.,80.]),true,1.2);assert!(cancel.load(Ordering::Acquire));assert!(app.object_finder.is_none());
    }
    #[test]
    fn bounded_memo_and_invalid_overlay_shapes_never_fabricate_success() {
        let mut points=Vec::new();points.try_reserve_exact(64).unwrap();
        for n in 0..100 {memo(&mut points,[n,0]).unwrap();}assert_eq!(points.len(),64);assert_eq!(points.first(),Some(&[36,0]));
        let malformed=Region {bbox:photocraft_geom::Rect::new(0,0,4,4),mask:vec![255;15]};assert!(overlay_image(&malformed,Color32::WHITE).is_err());
        let oversized=Region {bbox:photocraft_geom::Rect::new(i32::MIN,0,i32::MAX,1),mask:Vec::new()};assert!(overlay_image(&oversized,Color32::WHITE).is_err());
    }

}
