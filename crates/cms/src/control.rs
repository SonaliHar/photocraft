//! Local cooperative CMS control; no workspace/platform dependencies.
use crate::CmsError;
#[derive(Clone,Copy,Debug)]
pub struct Limits { pub resident_bytes:usize, pub work_units:u64 }
impl Default for Limits { fn default()->Self { Self { resident_bytes:256<<20, work_units:1_000_000_000 } } }
#[derive(Debug)]
pub enum Error { Cancelled, Limit(&'static str), Allocation, Cms(CmsError) }
impl From<CmsError> for Error { fn from(error:CmsError)->Self { Self::Cms(error) } }
impl std::fmt::Display for Error {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { match self {Self::Cancelled=>write!(f,"CMS operation cancelled"),Self::Limit(s)=>write!(f,"CMS limit: {s}"),Self::Allocation=>write!(f,"CMS allocation failed"),Self::Cms(e)=>std::fmt::Display::fmt(e,f)} } }
impl std::error::Error for Error {}
pub type Result<T>=std::result::Result<T,Error>;
pub(crate) struct Budget<'a> { pub(crate) limits:Limits, pub(crate) cancelled:&'a dyn Fn()->bool, resident:usize, work:u64 }
impl<'a> Budget<'a> {
 pub(crate) fn new(limits:Limits,cancelled:&'a dyn Fn()->bool)->Result<Self>{
  let max=Limits::default();if limits.resident_bytes>max.resident_bytes||limits.work_units>max.work_units{return Err(Error::Limit("limits may only be lowered"));}
  let b=Self{limits,cancelled,resident:0,work:0};b.poll()?;Ok(b)
 }
 pub(crate) fn poll(&self)->Result<()>{if (self.cancelled)(){Err(Error::Cancelled)}else{Ok(())}}
 pub(crate) fn admit(&self,bytes:usize,work:u64)->Result<()>{self.poll()?;if self.resident.checked_add(bytes).is_none_or(|n|n>self.limits.resident_bytes)||self.work.checked_add(work).is_none_or(|n|n>self.limits.work_units){return Err(Error::Limit("prospective CMS storage/work"));}Ok(())}
 pub(crate) fn retained(&mut self,n:usize)->Result<()>{self.admit(n,0)?;self.resident+=n;Ok(())}
 pub(crate) fn work(&mut self,n:u64)->Result<()>{self.admit(0,n)?;self.work+=n;Ok(())}
 pub(crate) fn reserve<T>(&mut self,n:usize)->Result<Vec<T>>{let bytes=n.checked_mul(std::mem::size_of::<T>()).ok_or(Error::Limit("vector size"))?;self.admit(bytes,0)?;let mut v=Vec::new();v.try_reserve_exact(n).map_err(|_|Error::Allocation)?;let actual=v.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(Error::Limit("vector capacity"))?;self.retained(actual)?;Ok(v)}
 pub(crate) fn copy<T:Copy>(&mut self,s:&[T])->Result<Vec<T>>{let mut v=self.reserve(s.len())?;for part in s.chunks(1024){self.work(part.len() as u64)?;v.extend_from_slice(part);}Ok(v)}
}
pub(crate) fn invalid(message:&str)->Error{CmsError::Invalid(message.into()).into()}
