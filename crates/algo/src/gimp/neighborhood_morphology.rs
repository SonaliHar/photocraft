//! Caller-defined channelwise rectangular extrema in native working samples.
//! No GEGL operation/version, circle shape, CMS transform, premultiplication,
//! rounding, serialized-source or stock-default admission is inferred here.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode,PixelFormat,SampleType,try_read_sample_f64};
use photocraft_geom::{Rect,TILE_SIZE,TileCoord};
use photocraft_raster::{Interrupt,MAX_CHECKED_RASTER_BYTES,RasterInputError,Surface,checked_region_byte_len,
    backing::{BackingError,ReadControl},checked_view::{CheckedRasterView,RasterViewError}};
use std::{cmp::Ordering,collections::VecDeque,mem::size_of};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Extremum { Minimum,Maximum }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Ties { First,Last }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Alpha { PreserveCenter,IndependentExtremum }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Abyss {
    Clamp,
    /// Exactly one native-endian pixel in the source format; no zero default.
    Constant { bytes:[u8;32],length:u8 },
}
#[derive(Clone,Copy,Debug)]
pub struct Params {
    pub radius_x:u32,pub radius_y:u32,pub extremum:Extremum,pub ties:Ties,
    pub alpha:Alpha,pub abyss:Abyss,pub source_context:WorkingContext,
}
#[derive(Clone,Copy,Debug)]
pub struct Limits {
    pub resident_bytes:u64,pub work_units:u64,
    /// Caller-selected maximum pixels in any single tile's rectangular halo.
    pub halo_pixels:u64,
}
#[derive(Clone,Debug,PartialEq,Eq)]
pub enum Error { Raster(RasterInputError),View(RasterViewError),ContextMismatch }
impl std::fmt::Display for Error {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {write!(f,"native rectangular extrema: {self:?}")}
}
impl std::error::Error for Error {}
impl From<RasterInputError> for Error {fn from(value:RasterInputError)->Self {Self::Raster(value)}}
impl From<RasterViewError> for Error {fn from(value:RasterViewError)->Self {
    if matches!(value,RasterViewError::Resident(RasterInputError::Cancelled)|RasterViewError::Control(BackingError::Cancelled)|RasterViewError::Backed(BackingError::Cancelled)) {Self::Raster(RasterInputError::Cancelled)} else {Self::View(value)}
}}
type Result<T>=std::result::Result<T,Error>;
fn poll(control:&Interrupt<'_>)->Result<()> {control.check().map_err(|_|RasterInputError::Cancelled.into())}
fn limit()->Error {RasterInputError::LimitExceeded.into()}
fn invalid()->Error {RasterInputError::InvalidBounds.into()}
fn rect(rect:Rect)->Result<()> {
    if rect.x1<rect.x0 || rect.y1<rect.y0 {return Err(invalid());}Ok(())
}
fn expand(rect:Rect,rx:u32,ry:u32)->Result<Rect> {
    let edge=|value:i32,amount:u32,subtract:bool|->Result<i32> {
        let value=if subtract {i64::from(value).checked_sub(i64::from(amount))} else {i64::from(value).checked_add(i64::from(amount))};
        i32::try_from(value.ok_or_else(invalid)?).map_err(|_|invalid())
    };
    Ok(Rect::new(edge(rect.x0,rx,true)?,edge(rect.y0,ry,true)?,edge(rect.x1,rx,false)?,edge(rect.y1,ry,false)?))
}
fn tiles(rect:Rect)->Result<u64> {
    if rect.is_empty() {return Ok(0);}
    let first=TileCoord::containing(rect.x0,rect.y0);
    let last=TileCoord::containing(rect.x1.checked_sub(1).ok_or_else(invalid)?,rect.y1.checked_sub(1).ok_or_else(invalid)?);
    for coordinate in [first,last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n|n.checked_add(TILE_SIZE)).ok_or_else(invalid)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n|n.checked_add(TILE_SIZE)).ok_or_else(invalid)?;
    }
    let width=u64::try_from(i64::from(last.tx)-i64::from(first.tx)+1).map_err(|_|invalid())?;
    let height=u64::try_from(i64::from(last.ty)-i64::from(first.ty)+1).map_err(|_|invalid())?;
    width.checked_mul(height).ok_or_else(limit)
}
fn finite(pixel:&[u8],format:PixelFormat)->Result<()> {
    if pixel.len()!=format.bytes_per_pixel() {return Err(RasterInputError::LengthMismatch.into());}
    for channel in 0..format.channels() {
        let value=try_read_sample_f64(pixel,format.sample,channel).map_err(|_|Error::Raster(RasterInputError::InvalidSample))?;
        if !value.is_finite() {return Err(RasterInputError::InvalidSample.into());}
    }
    Ok(())
}
#[derive(Clone,Copy)]
enum SampleValue { Code(u32),Float(f64) }
impl SampleValue {
    fn compare(self,other:Self)->Result<Ordering> {
        match (self,other) {
            (Self::Code(a),Self::Code(b))=>Ok(a.cmp(&b)),
            (Self::Float(a),Self::Float(b))=>a.partial_cmp(&b).ok_or_else(||RasterInputError::InvalidSample.into()),
            _=>Err(RasterInputError::LengthMismatch.into()),
        }
    }
}
#[derive(Clone,Copy)]
struct Entry {position:usize,value:SampleValue,raw:[u8;8]}
fn insert(queue:&mut VecDeque<Entry>,entry:Entry,window:usize,params:Params,control:&Interrupt<'_>)->Result<()> {
    let start=entry.position.saturating_sub(window.checked_sub(1).ok_or_else(limit)?);
    while queue.front().is_some_and(|old|old.position<start) {queue.pop_front();}
    let mut popped=0usize;
    while let Some(old)=queue.back() {
        let order=entry.value.compare(old.value)?;
        let better=match params.extremum {Extremum::Minimum=>order==Ordering::Less,Extremum::Maximum=>order==Ordering::Greater};
        if !better && !(params.ties==Ties::Last && order==Ordering::Equal) {break;}
        if popped.is_multiple_of(256) {poll(control)?;}queue.pop_back();popped=popped.checked_add(1).ok_or_else(limit)?;
    }
    if queue.len()>=window || queue.len()>=queue.capacity() {return Err(limit());}
    queue.push_back(entry);Ok(())
}
fn entry(pixel:&[u8],channel:usize,format:PixelFormat,position:usize)->Result<Entry> {
    let width=format.sample.bytes();let start=channel.checked_mul(width).ok_or_else(limit)?;
    let end=start.checked_add(width).ok_or_else(limit)?;
    let sample=pixel.get(start..end).ok_or(RasterInputError::LengthMismatch)?;
    let length=||Error::Raster(RasterInputError::LengthMismatch);
    let value=match format.sample {
        SampleType::U8=>SampleValue::Code(u32::from(*sample.first().ok_or_else(length)?)),
        SampleType::U16=>SampleValue::Code(u32::from(u16::from_ne_bytes(sample.try_into().map_err(|_|length())?))),
        SampleType::U32=>SampleValue::Code(u32::from_ne_bytes(sample.try_into().map_err(|_|length())?)),
        SampleType::F32|SampleType::F64=> {
            let value=try_read_sample_f64(sample,format.sample,0).map_err(|_|Error::Raster(RasterInputError::InvalidSample))?;
            if !value.is_finite() {return Err(RasterInputError::InvalidSample.into());}SampleValue::Float(value)
        }
    };
    let mut raw=[0;8];raw.get_mut(..width).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(sample);
    Ok(Entry {position,value,raw})
}
fn copy_sample(pixel:&mut [u8],channel:usize,sample:Entry,format:PixelFormat)->Result<()> {
    let width=format.sample.bytes();let start=channel.checked_mul(width).ok_or_else(limit)?;
    let end=start.checked_add(width).ok_or_else(limit)?;
    pixel.get_mut(start..end).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(sample.raw.get(..width).ok_or(RasterInputError::LengthMismatch)?);Ok(())
}
fn sample(view:&CheckedRasterView<'_>,extent:Rect,x:i32,y:i32,abyss:Abyss,raw:&mut [u8],read:&ReadControl<'_>)->Result<()> {
    if extent.contains(x,y) {view.read_logical_pixel_native_into(x,y,raw,read)?;}
    else {match abyss {
        Abyss::Clamp=> {
            let x=x.clamp(extent.x0,extent.x1.checked_sub(1).ok_or_else(invalid)?);
            let y=y.clamp(extent.y0,extent.y1.checked_sub(1).ok_or_else(invalid)?);
            view.read_logical_pixel_native_into(x,y,raw,read)?;
        }
        Abyss::Constant {bytes,length}=> {
            if usize::from(length)!=raw.len() {return Err(RasterInputError::LengthMismatch.into());}
            raw.copy_from_slice(bytes.get(..raw.len()).ok_or(RasterInputError::LengthMismatch)?);
        }
    }}
    finite(raw,view.format())
}
fn buffer(length:usize,control:&Interrupt<'_>)->Result<Vec<u8>> {
    poll(control)?;let mut out=Vec::new();out.try_reserve_exact(length).map_err(|_|Error::Raster(RasterInputError::AllocationFailed))?;
    if out.capacity()>length.checked_add(16).ok_or_else(limit)? {return Err(limit());}
    // Initialize in bounded bands rather than one uncancellable huge resize.
    let zero=[0;8192];while out.len()<length {poll(control)?;let amount=(length-out.len()).min(zero.len());out.extend_from_slice(zero.get(..amount).ok_or(RasterInputError::LengthMismatch)?);}
    Ok(out)
}
fn pixel_mut(bytes:&mut [u8],index:usize,bpp:usize)->Result<&mut [u8]> {
    let start=index.checked_mul(bpp).ok_or_else(limit)?;let end=start.checked_add(bpp).ok_or_else(limit)?;
    bytes.get_mut(start..end).ok_or_else(||RasterInputError::LengthMismatch.into())
}

/// Private COW output; only ROI pixels change, outside pixels/default bytes remain
/// raw. Each component independently selects an original native sample. Numeric
/// ties (including signed zero) use explicit row-major first/last selection.
/// Alpha is independent extrema or exact center bytes, never premultiplied.
///
/// Finiteness is checked for the default, explicit abyss pixel, every sampled
/// support component and preserved center alpha; unsampled pixels outside ROI
/// are retained raw. Working context equality is a caller assertion, not ICC/CMS
/// validation. Rectangle radii are integral and capped at 4096 in each axis.
pub fn try_rectangular_extrema(source:&Surface,roi:Rect,input_extent:Rect,params:Params,expected_context:WorkingContext,limits:Limits,control:&Interrupt<'_>)->Result<Surface> {
    poll(control)?;
    if params.source_context!=expected_context {return Err(Error::ContextMismatch);}
    let format=source.format();let bpp=format.bytes_per_pixel();let channels=format.channels();
    if !matches!(format.mode,ColorMode::Rgb|ColorMode::Grayscale) || !matches!(format.sample,SampleType::U8|SampleType::U16|SampleType::U32|SampleType::F32|SampleType::F64) || bpp>32 || channels>4 {return Err(RasterInputError::UnsupportedConversion.into());}
    if params.radius_x>4096 || params.radius_y>4096 || limits.resident_bytes==0 || limits.work_units==0 || limits.halo_pixels==0 {return Err(limit());}
    rect(roi)?;rect(input_extent)?;if input_extent.is_empty() {return Err(invalid());}
    checked_region_byte_len(roi,format)?;checked_region_byte_len(input_extent,format)?;
    let output_tiles=tiles(roi)?;let source_tiles=u64::try_from(source.tile_count()).map_err(|_|limit())?;
    let tile_side=u64::try_from(TILE_SIZE).map_err(|_|limit())?;
    let tile_bytes=tile_side.checked_mul(tile_side).and_then(|n|n.checked_mul(bpp as u64)).ok_or_else(limit)?;
    let width=u64::from(roi.width()).min(tile_side);let height=u64::from(roi.height()).min(tile_side);
    let halo_width=width.checked_add(u64::from(params.radius_x)*2).ok_or_else(limit)?;
    let halo_height=height.checked_add(u64::from(params.radius_y)*2).ok_or_else(limit)?;
    let halo_area=halo_width.checked_mul(halo_height).ok_or_else(limit)?;
    if !roi.is_empty() {expand(roi,params.radius_x,params.radius_y)?;}
    if halo_area>limits.halo_pixels {return Err(limit());}
    let filtered=if format.alpha && params.alpha==Alpha::PreserveCenter {channels.checked_sub(1).ok_or_else(limit)?} else {channels};
    let max_window=u64::from(params.radius_x.max(params.radius_y)).checked_mul(2).and_then(|n|n.checked_add(1)).ok_or_else(limit)?;
    let deque_bytes=max_window.checked_mul(2).and_then(|n|n.checked_mul(filtered as u64)).and_then(|n|n.checked_mul(size_of::<Entry>() as u64)).ok_or_else(limit)?;
    let scratch=width.checked_mul(halo_height).and_then(|n|n.checked_mul(bpp as u64))
        .and_then(|n|n.checked_add(width.checked_mul(height)?.checked_mul(bpp as u64)?)).and_then(|n|n.checked_add(deque_bytes)).and_then(|n|n.checked_add(8192)).ok_or_else(limit)?;
    let maps=source_tiles.checked_mul(512).ok_or_else(limit)?;
    let replacements=output_tiles.checked_mul(tile_bytes.checked_add(512).ok_or_else(limit)?).ok_or_else(limit)?;
    let peak=source.resident_accounted_bytes()?.checked_add(maps).and_then(|n|n.checked_add(replacements)).and_then(|n|n.checked_add(scratch)).and_then(|n|n.checked_add(tile_bytes.checked_mul(4)?)).ok_or_else(limit)?;
    if peak>limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {return Err(limit());}
    // Native view/read checks, finite decoding, amortized deque comparisons,
    // vertical samples and copied/written bytes are conservatively precharged.
    let per_tile=halo_area.checked_mul(64+(bpp as u64)*4+(channels as u64)*32)
        .and_then(|n|n.checked_add(width.checked_mul(halo_height)?.checked_mul((channels as u64)*32)?))
        .and_then(|n|n.checked_add(width.checked_mul(height)?.checked_mul(64+(bpp as u64)*4)?)).ok_or_else(limit)?;
    let work=output_tiles.checked_mul(per_tile).and_then(|n|n.checked_add(source_tiles.checked_mul(16)?))
        .and_then(|n|n.checked_add(output_tiles.checked_mul(source_tiles.checked_add(output_tiles)?)?.checked_mul(256)?)).ok_or_else(limit)?;
    let hard_work=if cfg!(target_arch="wasm32") {1_u64<<36} else {1_u64<<40};
    if work>limits.work_units.min(hard_work) {return Err(limit());}
    source.try_validate_tile_origins_with(control)?;
    for (_,tile) in source.tiles() {poll(control)?;if tile.bytes().len()!=usize::try_from(tile_bytes).map_err(|_|limit())? {return Err(RasterInputError::LengthMismatch.into());}}
    finite(source.default_bytes(),format)?;
    if let Abyss::Constant {bytes,length}=params.abyss {
        if usize::from(length)!=bpp {return Err(RasterInputError::LengthMismatch.into());}
        finite(bytes.get(..bpp).ok_or(RasterInputError::LengthMismatch)?,format)?;
    }
    let read=ReadControl {interrupt:control,work:&|_|Ok(())};
    let view=CheckedRasterView::resident_with_control(source,input_extent,&read)?;
    let mut output=source.clone();
    if roi.is_empty() {poll(control)?;return Ok(output);}
    for tile in roi.tiles() {
        poll(control)?;let part=tile.rect().intersect(&roi);let support=expand(part,params.radius_x,params.radius_y)?;
        let width=usize::try_from(part.width()).map_err(|_|limit())?;let height=usize::try_from(part.height()).map_err(|_|limit())?;
        let support_width=usize::try_from(support.width()).map_err(|_|limit())?;let support_height=usize::try_from(support.height()).map_err(|_|limit())?;
        let wx=usize::try_from(u64::from(params.radius_x)*2+1).map_err(|_|limit())?;let wy=usize::try_from(u64::from(params.radius_y)*2+1).map_err(|_|limit())?;
        let window=wx.max(wy);let mut queues:[VecDeque<Entry>;4]=std::array::from_fn(|_|VecDeque::new());
        for queue in queues.get_mut(..filtered).ok_or(RasterInputError::LengthMismatch)? {
            poll(control)?;queue.try_reserve_exact(window).map_err(|_|Error::Raster(RasterInputError::AllocationFailed))?;
            if queue.capacity()>window.checked_mul(2).ok_or_else(limit)? {return Err(limit());}
        }
        let mut horizontal=buffer(width.checked_mul(support_height).and_then(|n|n.checked_mul(bpp)).ok_or_else(limit)?,control)?;
        let mut encoded=buffer(width.checked_mul(height).and_then(|n|n.checked_mul(bpp)).ok_or_else(limit)?,control)?;
        for row in 0..support_height {
            poll(control)?;for queue in &mut queues {queue.clear();}
            let y=i32::try_from(i64::from(support.y0)+i64::try_from(row).map_err(|_|invalid())?).map_err(|_|invalid())?;
            for column in 0..support_width {
                if column.is_multiple_of(256) {poll(control)?;}
                let x=i32::try_from(i64::from(support.x0)+i64::try_from(column).map_err(|_|invalid())?).map_err(|_|invalid())?;
                let mut raw=[0;32];let pixel=raw.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;sample(&view,input_extent,x,y,params.abyss,pixel,&read)?;
                for (channel,queue) in queues.get_mut(..filtered).ok_or(RasterInputError::LengthMismatch)?.iter_mut().enumerate() {insert(queue,entry(pixel,channel,format,column)?,wx,params,control)?;}
                if column>=wx-1 {
                    let out_column=column-(wx-1);let index=row.checked_mul(width).and_then(|n|n.checked_add(out_column)).ok_or_else(limit)?;
                    let target=pixel_mut(&mut horizontal,index,bpp)?;
                    for (channel,queue) in queues.get(..filtered).ok_or(RasterInputError::LengthMismatch)?.iter().enumerate() {copy_sample(target,channel,*queue.front().ok_or(RasterInputError::LengthMismatch)?,format)?;}
                }
            }
        }
        for column in 0..width {
            poll(control)?;for queue in &mut queues {queue.clear();}
            for row in 0..support_height {
                if row.is_multiple_of(256) {poll(control)?;}
                let index=row.checked_mul(width).and_then(|n|n.checked_add(column)).ok_or_else(limit)?;
                let start=index.checked_mul(bpp).ok_or_else(limit)?;let end=start.checked_add(bpp).ok_or_else(limit)?;
                let pixel=horizontal.get(start..end).ok_or(RasterInputError::LengthMismatch)?;
                for (channel,queue) in queues.get_mut(..filtered).ok_or(RasterInputError::LengthMismatch)?.iter_mut().enumerate() {insert(queue,entry(pixel,channel,format,row)?,wy,params,control)?;}
                if row>=wy-1 {
                    let out_row=row-(wy-1);let index=out_row.checked_mul(width).and_then(|n|n.checked_add(column)).ok_or_else(limit)?;let target=pixel_mut(&mut encoded,index,bpp)?;
                    if format.alpha && params.alpha==Alpha::PreserveCenter {
                        let x=i32::try_from(i64::from(part.x0)+i64::try_from(column).map_err(|_|invalid())?).map_err(|_|invalid())?;
                        let y=i32::try_from(i64::from(part.y0)+i64::try_from(out_row).map_err(|_|invalid())?).map_err(|_|invalid())?;
                        sample(&view,input_extent,x,y,params.abyss,target,&read)?;
                    }
                    for (channel,queue) in queues.get(..filtered).ok_or(RasterInputError::LengthMismatch)?.iter().enumerate() {copy_sample(target,channel,*queue.front().ok_or(RasterInputError::LengthMismatch)?,format)?;}
                }
            }
        }
        poll(control)?;output.try_write_interleaved_with(part,&encoded,control)?;
    }
    poll(control)?;Ok(output)
}

// Synthetic mathematical/native-byte drafts, intentionally unrun. No GIMP oracle
// or source/version/palette/shape/profile qualification is claimed by these cases.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::try_write_sample_f64;
    use photocraft_doc::SampleEncoding;
    use std::sync::atomic::{AtomicUsize,Ordering};
    fn context()->WorkingContext {WorkingContext {encoding:SampleEncoding::Linear,profile_identity:[17;32]}}
    fn params()->Params {Params {radius_x:1,radius_y:1,extremum:Extremum::Maximum,ties:Ties::First,alpha:Alpha::PreserveCenter,abyss:Abyss::Clamp,source_context:context()}}
    fn limits()->Limits {Limits {resident_bytes:MAX_CHECKED_RASTER_BYTES,work_units:1<<36,halo_pixels:1<<20}}
    fn put(surface:&mut Surface,x:i32,y:i32,values:&[f64]) {
        let format=surface.format();let mut bytes=vec![0;format.bytes_per_pixel()];
        for (channel,value) in values.iter().enumerate() {try_write_sample_f64(&mut bytes,format.sample,channel,*value).unwrap();}
        surface.try_write_interleaved(Rect::new(x,y,x+1,y+1),&bytes).unwrap();
    }
    fn raw(surface:&Surface,x:i32,y:i32)->Vec<u8> {surface.try_read_interleaved(Rect::new(x,y,x+1,y+1)).unwrap()}
    #[test]
    fn each_depth_copies_winning_native_gray_and_exact_center_alpha_across_tile_edge() {
        for depth in SampleType::ALL {
            let format=PixelFormat::new(ColorMode::Grayscale,depth,true);let mut source=Surface::new(format);let input=Rect::new(255,-1,258,1);
            for (x,y,color,alpha) in [(255,-1,0.2,1.0),(256,-1,0.4,0.1),(257,-1,0.8,0.8),(255,0,0.9,0.9),(256,0,0.1,0.2),(257,0,0.6,0.7)] {put(&mut source,x,y,&[color,alpha]);}
            let retained=source.clone();let roi=Rect::new(256,0,257,1);let result=try_rectangular_extrema(&source,roi,input,params(),context(),limits(),&Interrupt::NONE).unwrap();
            let width=depth.bytes();let mut expected=raw(&source,256,0);expected.get_mut(..width).unwrap().copy_from_slice(raw(&source,255,0).get(..width).unwrap());
            assert_eq!(raw(&result,256,0),expected);assert_eq!(raw(&result,255,-1),raw(&source,255,-1));assert_eq!(result.default_bytes(),source.default_bytes());assert_eq!(source,retained);
            let mut independent=params();independent.alpha=Alpha::IndependentExtremum;
            let result=try_rectangular_extrema(&source,roi,input,independent,context(),limits(),&Interrupt::NONE).unwrap();
            expected.get_mut(width..).unwrap().copy_from_slice(raw(&source,255,-1).get(width..).unwrap());assert_eq!(raw(&result,256,0),expected);
        }
    }
    #[test]
    fn u32_adjacent_samples_above_f32_precision_remain_distinct() {
        let mut source=Surface::new(PixelFormat::new(ColorMode::Grayscale,SampleType::U32,false));
        let bytes=[16_777_216u32,16_777_217,16_777_218].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();source.try_write_interleaved(Rect::new(-1,0,2,1),&bytes).unwrap();
        let mut p=params();p.radius_y=0;let result=try_rectangular_extrema(&source,Rect::new(0,0,1,1),Rect::new(-1,0,2,1),p,context(),limits(),&Interrupt::NONE).unwrap();
        assert_eq!(raw(&result,0,0),16_777_218u32.to_ne_bytes());
        p.extremum=Extremum::Minimum;let result=try_rectangular_extrema(&source,Rect::new(0,0,1,1),Rect::new(-1,0,2,1),p,context(),limits(),&Interrupt::NONE).unwrap();assert_eq!(raw(&result,0,0),16_777_216u32.to_ne_bytes());
    }
    #[test]
    fn floating_numeric_ties_preserve_explicit_first_last_signed_zero_bits() {
        for depth in [SampleType::F32,SampleType::F64] {
            let mut source=Surface::new(PixelFormat::new(ColorMode::Grayscale,depth,false));
            let bytes=if depth==SampleType::F32 {[0u32,0x80000000,0x80000000].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()} else {[0u64,0x8000000000000000,0x8000000000000000].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()};
            source.try_write_interleaved(Rect::new(-1,0,2,1),&bytes).unwrap();let mut p=params();p.radius_y=0;
            for extrema in [Extremum::Minimum,Extremum::Maximum] {p.extremum=extrema;p.ties=Ties::First;let first=try_rectangular_extrema(&source,Rect::new(0,0,1,1),Rect::new(-1,0,2,1),p,context(),limits(),&Interrupt::NONE).unwrap();assert_eq!(raw(&first,0,0),raw(&source,-1,0));
                p.ties=Ties::Last;let last=try_rectangular_extrema(&source,Rect::new(0,0,1,1),Rect::new(-1,0,2,1),p,context(),limits(),&Interrupt::NONE).unwrap();assert_eq!(raw(&last,0,0),raw(&source,1,0));}
        }
    }
    #[test]
    fn explicit_constant_abyss_is_not_a_sparse_zero_default() {
        let mut source=Surface::new(PixelFormat::new(ColorMode::Grayscale,SampleType::U16,false));source.try_write_interleaved(Rect::new(0,0,1,1),&10u16.to_ne_bytes()).unwrap();
        let mut p=params();p.radius_y=0;p.extremum=Extremum::Minimum;let bounds=Rect::new(0,0,1,1);
        let clamp=try_rectangular_extrema(&source,bounds,bounds,p,context(),limits(),&Interrupt::NONE).unwrap();assert_eq!(raw(&clamp,0,0),10u16.to_ne_bytes());
        let mut bytes=[0;32];bytes.get_mut(..2).unwrap().copy_from_slice(&7u16.to_ne_bytes());p.abyss=Abyss::Constant {bytes,length:2};
        let constant=try_rectangular_extrema(&source,bounds,bounds,p,context(),limits(),&Interrupt::NONE).unwrap();assert_eq!(raw(&constant,0,0),7u16.to_ne_bytes());
    }
    #[test]
    fn separable_rectangle_matches_independent_small_brute_force_samples() {
        let format=PixelFormat::new(ColorMode::Rgb,SampleType::F64,true);let mut source=Surface::new(format);let input=Rect::new(-2,254,3,259);let roi=Rect::new(-1,255,2,258);
        for y in input.y0..input.y1 {for x in input.x0..input.x1 {put(&mut source,x,y,&[f64::from(x*7-y),f64::from(y*3+x),f64::from(x*x-y),0.5]);}}
        for extrema in [Extremum::Minimum,Extremum::Maximum] {
            let mut p=params();p.extremum=extrema;p.alpha=Alpha::IndependentExtremum;
            let result=try_rectangular_extrema(&source,roi,input,p,context(),limits(),&Interrupt::NONE).unwrap();
            for y in roi.y0..roi.y1 {for x in roi.x0..roi.x1 {
                let mut expected=[0u8;32];for channel in 0..4 {
                    let mut winner:Option<(f64,Vec<u8>)>=None;
                    for sy in y-1..=y+1 {for sx in x-1..=x+1 {
                        let candidate=raw(&source,sx.clamp(input.x0,input.x1-1),sy.clamp(input.y0,input.y1-1));let value=try_read_sample_f64(&candidate,SampleType::F64,channel).unwrap();
                        if winner.as_ref().is_none_or(|old|match extrema {Extremum::Minimum=>value<old.0,Extremum::Maximum=>value>old.0}) {winner=Some((value,candidate));}
                    }}
                    let bytes=winner.unwrap().1;expected.get_mut(channel*8..channel*8+8).unwrap().copy_from_slice(bytes.get(channel*8..channel*8+8).unwrap());
                }
                assert_eq!(raw(&result,x,y),expected);
            }}
        }
    }
    #[test]
    fn nonfinite_sample_and_context_budget_abyss_shape_refuse_without_source_changes() {
        let format=PixelFormat::new(ColorMode::Grayscale,SampleType::F64,false);let mut source=Surface::new(format);let bounds=Rect::new(0,0,1,1);
        source.try_write_interleaved(bounds,&f64::INFINITY.to_ne_bytes()).unwrap();let before=source.clone();
        assert!(matches!(try_rectangular_extrema(&source,bounds,bounds,params(),context(),limits(),&Interrupt::NONE),Err(Error::Raster(RasterInputError::InvalidSample))));assert_eq!(source,before);
        source.try_write_interleaved(bounds,&2.0f64.to_ne_bytes()).unwrap();let before=source.clone();let mut mismatch=context();mismatch.profile_identity=[18;32];
        assert!(matches!(try_rectangular_extrema(&source,bounds,bounds,params(),mismatch,limits(),&Interrupt::NONE),Err(Error::ContextMismatch)));
        for restricted in [Limits {resident_bytes:1,..limits()},Limits {work_units:1,..limits()},Limits {halo_pixels:1,..limits()}] {assert!(try_rectangular_extrema(&source,bounds,bounds,params(),context(),restricted,&Interrupt::NONE).is_err());}
        let mut p=params();p.radius_x=u32::MAX;assert!(try_rectangular_extrema(&source,bounds,bounds,p,context(),limits(),&Interrupt::NONE).is_err());p=params();p.abyss=Abyss::Constant {bytes:[0;32],length:1};assert!(try_rectangular_extrema(&source,bounds,bounds,p,context(),limits(),&Interrupt::NONE).is_err());assert_eq!(source,before);
    }
    #[test]
    fn cancellation_discards_partial_roi_and_preserves_original_tile_bytes() {
        let format=PixelFormat::new(ColorMode::Grayscale,SampleType::U8,false);let mut source=Surface::new(format);let input=Rect::new(255,-1,259,3);let roi=Rect::new(256,0,258,2);
        for y in input.y0..input.y1 {for x in input.x0..input.x1 {put(&mut source,x,y,&[f64::from((x+y).rem_euclid(17))/16.0]);}}
        let before=source.clone();let count=AtomicUsize::new(0);let measure=||{count.fetch_add(1,Ordering::Relaxed);false};try_rectangular_extrema(&source,roi,input,params(),context(),limits(),&Interrupt::cancel_only(&measure)).unwrap();assert!(count.load(Ordering::Relaxed)>20);
        let calls=AtomicUsize::new(0);let stop=||calls.fetch_add(1,Ordering::Relaxed)>=20;
        assert!(matches!(try_rectangular_extrema(&source,roi,input,params(),context(),limits(),&Interrupt::cancel_only(&stop)),Err(Error::Raster(RasterInputError::Cancelled))));assert_eq!(source,before);
    }
}
