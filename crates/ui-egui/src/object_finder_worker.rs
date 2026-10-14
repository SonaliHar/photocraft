//! Private Object Finder task boundary. Native work receives a real token;
//! wasm remains inline (the token is polled, but no event-loop yielding is claimed).
use crate::PhotocraftApp;
use photocraft_doc::{DocId,LayerId};
use photocraft_engine::{WeakDocumentSnapshot,channel_cmds::ChannelView,
    object_finder_controlled::{FinderTarget,PreparedFinder}};
use photocraft_raster::Interrupt;
use std::sync::{Arc,atomic::{AtomicBool,Ordering},mpsc::{self,Receiver,TryRecvError}};
#[derive(Clone)]
pub(super) struct Frame {
    document:DocId,revision:u64,snapshot:WeakDocumentSnapshot,layer:Option<LayerId>,view:ChannelView,shell_mask:bool,all:bool,
}
fn failure()->String {tl!("Object Finder could not prepare a preview here. Move the pointer to retry.").to_owned()}
impl Frame {
    pub(super) fn capture(app:&PhotocraftApp)->Result<Self,String> {
        let state=app.session.active().ok_or_else(failure)?;
        fn copy(values:&[bool],cap:usize)->Result<Vec<bool>,String> {
            if values.len()>cap {return Err(failure());}
            let mut output=Vec::new();output.try_reserve_exact(values.len()).map_err(|_|failure())?;output.extend_from_slice(values);Ok(output)
        }
        let view=&state.channel_view;
        Ok(Self {document:state.doc.id,revision:state.revision,snapshot:state.doc.downgrade(),layer:state.active_layer,
            view:ChannelView {target:view.target,color_hidden:copy(&view.color_hidden,16)?,alpha_visible:copy(&view.alpha_visible,10_000)?,
                quick_mask_hidden:view.quick_mask_hidden,layer_mask:view.layer_mask,editing_mask:view.editing_mask},
            shell_mask:app.layer_mask_targeted(),all:app.ui.tool_options.sample_all_layers})
    }
    pub(super) fn current(&self,app:&PhotocraftApp)->bool {
        app.session.active().is_some_and(|state|state.doc.id==self.document && state.revision==self.revision && self.snapshot.is_current(&state.doc)
            && state.active_layer==self.layer && state.channel_view==self.view && app.layer_mask_targeted()==self.shell_mask
            && app.ui.tool_options.sample_all_layers==self.all)
    }
}
pub(super) enum Outcome {Ready(PreparedFinder),Cancelled,Failed}
pub(super) struct Job {pub(super) at:[i32;2],cancel:Arc<AtomicBool>,rx:Receiver<Outcome>}
impl Drop for Job {fn drop(&mut self){self.cancel.store(true,Ordering::Release);}}
impl Job {
    pub(super) fn poll(&self)->Option<Outcome> {
        match self.rx.try_recv() {Ok(outcome)=>Some(outcome),Err(TryRecvError::Empty)=>None,Err(TryRecvError::Disconnected)=>Some(Outcome::Failed)}
    }
}
pub(super) fn spawn(target:FinderTarget,at:[i32;2])->Result<Job,String> {
    let (tx,rx)=mpsc::sync_channel(1);let cancel=Arc::new(AtomicBool::new(false));let flag=Arc::clone(&cancel);
    let work=move|| {
        let cancelled=||flag.load(Ordering::Acquire);let control=Interrupt::cancel_only(&cancelled);
        // App-shell safety boundary: escaped provider panics are failures, never
        // empty objects or cached successful misses. Partial stages remain private.
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||photocraft_engine::object_finder_controlled::prepare(target,&control)));
        let outcome=if cancelled(){Outcome::Cancelled}else{match result {Ok(Ok(prepared))=>Outcome::Ready(prepared),_=>Outcome::Failed}};
        let _=tx.try_send(outcome);
    };
    #[cfg(not(target_arch="wasm32"))]
    {let _worker=std::thread::Builder::new().name("object-finder".into()).spawn(work).map_err(|_|failure())?;}
    #[cfg(target_arch="wasm32")]
    work();
    Ok(Job {at,cancel,rx})
}
#[cfg(test)]
pub(super) fn test_job(at:[i32;2],failed:bool)->(Job,Arc<AtomicBool>) {
    let (sender,rx)=mpsc::sync_channel(1);let cancel=Arc::new(AtomicBool::new(false));
    if failed {assert!(sender.try_send(Outcome::Failed).is_ok());}
    (Job {at,cancel:Arc::clone(&cancel),rx},cancel)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_job_sets_real_token_without_publishing_result() {
        let (_sender,rx)=mpsc::sync_channel(1);let cancel=Arc::new(AtomicBool::new(false));
        let job=Job {at:[1,2],cancel:Arc::clone(&cancel),rx};drop(job);assert!(cancel.load(Ordering::Acquire));
    }
}
