//! Full CPU redraws of the GPU canvas off the UI thread (#4024). When the wgpu compositor can't
//! draw a full refresh (the layers exceed its memory budget, or the document uses something it
//! doesn't cover), the CPU compositor redraws the whole document: minutes on a multi-gigabyte
//! PSB. With background jobs (the desktop app) a worker composites it in blocks instead, the
//! visible ones first, and each frame uploads the blocks that are done, so the window keeps
//! drawing and answering while the rest of the canvas fills in. Until a block lands the texture
//! keeps what it showed before.
//!
//! An edit while it runs restarts the worker on the new document with the blocks still to draw;
//! the edit's own damage is drawn by the canvas as usual. A full change (a new preview, a new
//! display transform) starts over. Without background jobs (web, tests) the canvas keeps the
//! synchronous [`GpuCanvas::upload_composite`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};

use photocraft_doc::DocId;
use photocraft_engine::DocumentSnapshot;
use photocraft_engine::display_color::CanvasDisplay;
use photocraft_geom::Rect;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

use crate::PhotocraftApp;
use crate::gpu_canvas::GpuCanvas;

/// Side of a block in document pixels: whole render tiles, and a power of two so each block's
/// mip levels don't depend on its neighbours.
const BLOCK: i32 = 1024;

/// Blocks composited at once (in parallel, on top of each block's own tiles), and how many
/// finished blocks may wait for the UI thread: bounds the memory a redraw holds.
const BATCH: usize = 4;

/// One finished block, as texels of the canvas texture.
enum Piece {Block {block:usize,texels:Vec<u8>},Failed(String)}

/// A redraw in progress for one document's GPU texture.
pub(crate) struct Redraw {
    blocks: Vec<Rect>,
    pending: Vec<bool>,
    left: usize,
    /// Pixels uploaded so far.
    drawn: u64,
    rx: Receiver<Piece>,
    stop: Arc<AtomicBool>,
    started_ms: f64,
    fallback: Option<String>,
    key:(u64,u64),
    _source:DocumentSnapshot,
}

impl Redraw {
    /// Share of the document drawn, 0..=1.
    pub fn fraction(&self) -> f32 {
        if self.blocks.is_empty() { 1.0 } else { 1.0 - self.left as f32 / self.blocks.len() as f32 }
    }
}

impl Drop for Redraw {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Whether full CPU refreshes run in the background here.
pub(crate) fn enabled(app: &PhotocraftApp) -> bool {
    cfg!(not(target_arch = "wasm32")) && app.background_jobs
}

/// `bounds` cut into [`BLOCK`]-sized blocks, row by row.
fn blocks(bounds:Rect)->Result<Vec<Rect>,String> {
    if bounds.is_empty() || bounds.x0<0 || bounds.y0<0 {return Err("Invalid background redraw bounds".into());}
    let width=u64::try_from(i64::from(bounds.x1)-i64::from(bounds.x0)).map_err(|_|"Invalid redraw width")?;
    let height=u64::try_from(i64::from(bounds.y1)-i64::from(bounds.y0)).map_err(|_|"Invalid redraw height")?;
    let count=width.div_ceil(BLOCK as u64).checked_mul(height.div_ceil(BLOCK as u64)).filter(|count|*count<=100_000).and_then(|count|usize::try_from(count).ok()).ok_or("Background redraw block limit")?;
    let mut out=Vec::new();out.try_reserve_exact(count).map_err(|_|"Background redraw index allocation failed")?;
    let mut y=bounds.y0;
    while y<bounds.y1 {
        let end_y=y.saturating_add(BLOCK).min(bounds.y1);
        let mut x=bounds.x0;
        while x<bounds.x1 {
            let end_x=x.saturating_add(BLOCK).min(bounds.x1);
            out.push(Rect::new(x,y,end_x,end_y));x=end_x;
        }
        y=end_y;
    }
    Ok(out)
}
/// Visible blocks first, then nearest the view centre, with original row order breaking ties.
fn order(blocks:&[Rect],wanted:impl Iterator<Item=usize>,visible:Rect)->Result<Vec<usize>,String> {
    if blocks.len()>100_000 {return Err("Background redraw index limit".into());}
    let mut out=Vec::new();out.try_reserve_exact(blocks.len()).map_err(|_|"Background redraw order allocation failed")?;
    for index in wanted {
        if index>=blocks.len() || out.len()>=blocks.len() {return Err("Invalid background redraw block index".into());}
        out.push(index);
    }
    let (cx,cy)=((i128::from(visible.x0)+i128::from(visible.x1))/2,(i128::from(visible.y0)+i128::from(visible.y1))/2);
    out.sort_unstable_by_key(|index| {
        let Some(b)=blocks.get(*index) else {return (true,i128::MAX,*index)};
        let (bx,by)=((i128::from(b.x0)+i128::from(b.x1))/2,(i128::from(b.y0)+i128::from(b.y1))/2);
        (b.intersect(&visible).is_empty(),(bx-cx).pow(2)+(by-cy).pow(2),*index)
    });
    Ok(out)
}

/// Redraw `region` of document `doc` (`doc.id`'s texture, prepared by
/// [`GpuCanvas::refresh_with`]) in the background. A redraw already running for it continues on
/// `doc` with `region`'s blocks added.
#[allow(clippy::too_many_arguments)]
pub(crate) fn start(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    gpu: &GpuCanvas,
    doc: DocumentSnapshot,
    display: Option<Arc<CanvasDisplay>>,
    visible: Rect,
    region: Rect,
    fallback: Option<String>,
    key:(u64,u64),
) ->Result<(),String> {
    if doc.size.width>i32::MAX as u32 || doc.size.height>i32::MAX as u32 {return Err("Background redraw dimension limit".into());}
    let all = blocks(doc.bounds())?;
    let (mut pending, since, fallback) = match app.redraws.remove(&doc.id) {
        Some(old) if old.blocks == all => (clone_pending(&old.pending)?, (old.started_ms, old.drawn), fallback.or_else(|| old.fallback.clone())),
        _ => (empty_pending(all.len())?, (crate::gpu_canvas::now_ms(), 0), fallback),
    };
    for (p, b) in pending.iter_mut().zip(&all) {
        *p |= !b.intersect(&region).is_empty();
    }
    if pending.contains(&true) {
        spawn(app, ctx, gpu, doc, display, visible, all, pending, since, fallback,key)?;
    }
    Ok(())
}
fn empty_pending(count:usize)->Result<Vec<bool>,String> {let mut out=Vec::new();out.try_reserve_exact(count).map_err(|_|"Background redraw pending allocation failed")?;out.resize(count,false);Ok(out)}
fn clone_pending(source:&[bool])->Result<Vec<bool>,String> {let mut out=empty_pending(source.len())?;out.copy_from_slice(source);Ok(out)}

/// After an edit drew its own damage: continue document `doc.id`'s redraw, if one is running, on
/// the edited document, so blocks still to come don't bring back what was there before it.
pub(crate) fn restart(app: &mut PhotocraftApp, ctx: &egui::Context, gpu: &GpuCanvas, doc: DocumentSnapshot, display: Option<Arc<CanvasDisplay>>, visible: Rect,key:(u64,u64)) ->Result<(),String> {
    if app.redraws.contains_key(&doc.id) {
        start(app, ctx, gpu, doc, display, visible, Rect::EMPTY, None,key)?;
    }
    Ok(())
}

/// Upload the blocks document `doc`'s redraw has finished since the last frame. When the last one
/// lands, the redraw is recorded like the synchronous refresh it replaces (and explained once if
/// it was slow).
pub(crate) fn poll(app:&mut PhotocraftApp,gpu:&GpuCanvas,doc:DocId) {
    let mut failure=None;
    let Some(r)=app.redraws.get_mut(&doc) else {return};
    loop {
        match r.rx.try_recv() {
            Ok(Piece::Failed(error))=>{failure=Some(error);break;},
            Ok(Piece::Block {block,texels})=>{
                let Some(bounds)=r.blocks.get(block).copied() else {failure=Some("Invalid redraw block reply".into());break};
                let Some(pending)=r.pending.get_mut(block) else {failure=Some("Invalid redraw pending reply".into());break};
                let (Ok(x),Ok(y))=(u32::try_from(bounds.x0),u32::try_from(bounds.y0)) else {failure=Some("Invalid redraw upload origin".into());break};
                if *pending {
                    if !gpu.upload_rect(doc.0,[x,y],[bounds.width(),bounds.height()],&texels) {failure=Some("Background redraw texture upload failed".into());break;}
                    *pending=false;r.left=r.left.saturating_sub(1);
                    r.drawn=r.drawn.saturating_add(u64::from(bounds.width())*u64::from(bounds.height()));
                }
            },
            Err(TryRecvError::Empty)=>return,
            Err(TryRecvError::Disconnected)=>break,
        }
    }
    let Some(r)=app.redraws.remove(&doc) else {return};
    if let Some(error)=failure.or_else(||(r.left>0).then(||"Background redraw stopped before completion".to_owned())) {
        crate::canvas::background_redraw_failed(app,doc,r.key,error);return;
    }
    let ms=crate::gpu_canvas::now_ms()-r.started_ms;
    app.perf.record("background",r.drawn,ms,0.0);
    crate::notices::slow_cpu_refresh(app,doc.0,ms,r.fallback.as_deref());
}

/// The active document's redraw in the status bar: "Redrawing" and how much is done.
pub(crate) fn status(app: &PhotocraftApp, ui: &mut egui::Ui) {
    let Some(r) = app.session.active().and_then(|st| app.redraws.get(&st.doc.id)) else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(format!("{:.0}%", r.fraction() * 100.0)).color(t.text_dim).size(11.5).monospace());
    ui.label(egui::RichText::new(tl!("Redrawing")).color(t.text_dim).size(12.0));
}

/// Stop document `doc`'s redraw (its canvas was redrawn some other way, or is gone).
pub(crate) fn cancel(app: &mut PhotocraftApp, doc: DocId) {
    app.redraws.remove(&doc);
}

/// Stop redraws of documents that are no longer open.
pub(crate) fn retain(app: &mut PhotocraftApp) {
    let docs = app.session.documents();
    app.redraws.retain(|id, _| docs.iter().any(|st| st.doc.id == *id));
}

#[allow(clippy::too_many_arguments)]
fn spawn(app:&mut PhotocraftApp,ctx:&egui::Context,gpu:&GpuCanvas,doc:DocumentSnapshot,display:Option<Arc<CanvasDisplay>>,visible:Rect,blocks:Vec<Rect>,pending:Vec<bool>,(started_ms,drawn):(f64,u64),fallback:Option<String>,key:(u64,u64))->Result<(),String> {
    let id=doc.id;
    let format=gpu.prepare_texture(id.0,&doc).ok_or("Background redraw texture preparation failed")?;
    let todo=order(&blocks,(0..blocks.len()).filter(|index|pending.get(*index)==Some(&true)),visible)?;
    let left=todo.len();
    let mut worker_blocks=Vec::new();worker_blocks.try_reserve_exact(blocks.len()).map_err(|_|"Background redraw worker index allocation failed")?;worker_blocks.extend_from_slice(&blocks);
    app.redraws.try_reserve(1).map_err(|_|"Background redraw state allocation failed")?;
    let source=doc.clone();
    let (tx,rx)=std::sync::mpsc::sync_channel(BATCH);
    let stop=Arc::new(AtomicBool::new(false));
    let worker_stop=stop.clone();let ctx=ctx.clone();
    std::thread::Builder::new().name("canvas-redraw".into()).spawn(move|| {
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||draw(&doc,display.as_deref(),format,&worker_blocks,&todo,&worker_stop,&tx,&ctx)));
        let error=match result {Ok(Ok(()))=>None,Ok(Err(error))=>Some(error),Err(_)=>Some("Background redraw failed unexpectedly".into())};
        if !worker_stop.load(Ordering::Relaxed) && let Some(error)=error {let _=tx.send(Piece::Failed(error));}
        ctx.request_repaint();
    }).map_err(|error|format!("Could not start background redraw: {error}"))?;
    app.redraws.insert(id,Redraw {blocks,pending,left,drawn,rx,stop,started_ms,fallback,key,_source:source});
    Ok(())
}
/// Render at most one fixed-size batch at a time, preserving ordered block publication.
#[allow(clippy::too_many_arguments)]
fn draw(doc:&DocumentSnapshot,display:Option<&CanvasDisplay>,format:eframe::wgpu::TextureFormat,blocks:&[Rect],todo:&[usize],stop:&AtomicBool,tx:&SyncSender<Piece>,ctx:&egui::Context)->Result<(),String> {
    let cancel=||stop.load(Ordering::Relaxed);
    let control=photocraft_raster::Interrupt::cancel_only(&cancel);
    for batch in todo.chunks(BATCH) {
        control.check().map_err(|error|error.to_string())?;
        let mut pieces=Vec::new();pieces.try_reserve_exact(batch.len()).map_err(|_|"Background redraw batch allocation failed")?;pieces.resize_with(batch.len(),||None);
        #[cfg(not(target_arch="wasm32"))]
        let jobs=pieces.par_iter_mut().zip(batch.par_iter());
        #[cfg(target_arch="wasm32")]
        let jobs=pieces.iter_mut().zip(batch.iter());
        jobs.for_each(|(out,index)| {
            *out=Some((|| {
                let bounds=blocks.get(*index).copied().ok_or("Invalid background redraw work index")?;
                let buffer=photocraft_compose::render_with_interrupt(doc,bounds,&control).map_err(|error|error.to_string())?;
                let texels=crate::gpu_canvas::try_display_texels(format,display,&buffer,&control)?;
                Ok::<_,String>(Piece::Block {block:*index,texels})
            })());
        });
        for piece in pieces {
            control.check().map_err(|error|error.to_string())?;
            let piece=piece.ok_or("Missing background redraw block result")??;
            if tx.send(piece).is_err() {return Ok(());}
            ctx.request_repaint();
        }
    }
    Ok(())
}

#[cfg(test)]
#[path="background_redraw_tests.rs"]
mod tests;
