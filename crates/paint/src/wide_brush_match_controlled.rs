//! Borrowed, cancellable equality with the existing BrushSettings semantics.
//! Stored bitmaps compare their owner key and declared size, not resident data.
use crate::brush::{BrushSettings, Pattern, TipShape};
use crate::tile::{GrayTile, StoredTile};
use photocraft_raster::{Interrupt, RasterInputError};
type Result<T> = std::result::Result<T, RasterInputError>;
struct Comparison<'a, 'b> {
    control: &'a Interrupt<'b>,
    work: u64,
}
impl Comparison<'_, '_> {
    fn tick(&mut self, n: u64) -> Result<()> {
        self.control.check().map_err(|_| RasterInputError::Cancelled)?;
        self.work = self.work.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn bytes(&mut self, a: &[u8], b: &[u8]) -> Result<bool> {
        self.tick(1)?;
        if a.len() != b.len() {
            return Ok(false);
        }
        for (left, right) in a.chunks(256).zip(b.chunks(256)) {
            self.tick(left.len() as u64)?;
            if left != right {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn gray(&mut self, a: &GrayTile, b: &GrayTile) -> Result<bool> {
        self.tick(1)?;
        if a.width != b.width || a.height != b.height || a.data.len() != b.data.len() {
            return Ok(false);
        }
        for (left, right) in a.data.chunks(256).zip(b.data.chunks(256)) {
            self.tick(left.len() as u64)?;
            if left != right {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn stored(&mut self, a: &StoredTile, b: &StoredTile) -> Result<bool> {
        self.tick(1)?;
        if a.width != b.width || a.height != b.height {
            return Ok(false);
        }
        self.bytes(a.key.as_bytes(), b.key.as_bytes())
    }
    fn tip(&mut self, a: &TipShape, b: &TipShape) -> Result<bool> {
        self.tick(1)?;
        match (a, b) {
            (TipShape::Round, TipShape::Round) => Ok(true),
            (TipShape::Sampled(a), TipShape::Sampled(b)) => self.gray(a, b),
            (TipShape::Stored(a), TipShape::Stored(b)) => self.stored(a, b),
            _ => Ok(false),
        }
    }
    fn pattern(&mut self, a: &Pattern, b: &Pattern) -> Result<bool> {
        self.tick(1)?;
        match (a, b) {
            (Pattern::Procedural { style: a, size: asize, seed: aseed }, Pattern::Procedural { style: b, size: bsize, seed: bseed }) => {
                Ok(a == b && asize == bsize && aseed == bseed)
            }
            (Pattern::Tile(a), Pattern::Tile(b)) => self.gray(a, b),
            (Pattern::Stored(a), Pattern::Stored(b)) => self.stored(a, b),
            _ => Ok(false),
        }
    }
}
pub(super) fn equal(a: &BrushSettings, b: &BrushSettings, work: u64, control: &Interrupt<'_>) -> Result<bool> {
    let mut compare = Comparison { control, work };
    compare.tick(128)?;
    macro_rules! fields{($left:expr,$right:expr;$($field:ident),+)=>{true$(&&$left.$field==$right.$field)+};}
    if !fields!(a,b;size,hardness,spacing,spacing_enabled,opacity,flow,pressure_size,pressure_opacity,color,color_cmyk,background,erase,mode,angle,roundness,flip_x,flip_y,aliased,square,shape_dynamics,scattering,color_dynamics,transfer,pose,noise,wet_edges,build_up,build_up_rate,smoothing,protect_texture,locks,mixer,seed)
    {
        return Ok(false);
    }
    if !fields!(&a.texture,&b.texture;enabled,invert,scale,brightness,contrast,each_tip,mode,depth,depth_jitter) {
        return Ok(false);
    }
    if !fields!(&a.dual_brush,&b.dual_brush;enabled,mode,size,hardness,roundness,angle,spacing,scatter,both_axes,count,flip) {
        return Ok(false);
    }
    if !compare.tip(&a.tip, &b.tip)? || !compare.tip(&a.dual_brush.tip, &b.dual_brush.tip)? || !compare.pattern(&a.texture.pattern, &b.texture.pattern)? {
        return Ok(false);
    }
    compare.tick(1)?;
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sampled_and_stored_equality_keeps_existing_semantics_and_polls() {
        let mut a = BrushSettings { tip: TipShape::Sampled(GrayTile { width: 1024, height: 1, data: vec![7; 1024].into() }), ..Default::default() };
        let mut b = a.clone();
        assert_eq!(equal(&a, &b, 10000, &Interrupt::NONE).unwrap(), a == b);
        if let TipShape::Sampled(tile) = &mut b.tip {
            *std::sync::Arc::make_mut(&mut tile.data).get_mut(900).unwrap() = 8;
        }
        assert_eq!(equal(&a, &b, 10000, &Interrupt::NONE).unwrap(), a == b);
        let count = std::sync::atomic::AtomicUsize::new(0);
        let stop = || {
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            count.load(std::sync::atomic::Ordering::Relaxed) > 4
        };
        assert!(matches!(equal(&a, &a, 10000, &Interrupt::cancel_only(&stop)), Err(RasterInputError::Cancelled)));
        let stored = StoredTile {
            key: "owned-key".into(),
            width: 1024,
            height: 1,
            preview: std::sync::Arc::new(GrayTile { width: 1, height: 1, data: vec![0].into() }),
            full: None,
        };
        a.tip = TipShape::Stored(stored.clone());
        b.tip = TipShape::Stored(StoredTile { preview: std::sync::Arc::new(GrayTile { width: 1, height: 1, data: vec![65535].into() }), ..stored });
        assert_eq!(equal(&a, &b, 10000, &Interrupt::NONE).unwrap(), a == b);
    }
}
