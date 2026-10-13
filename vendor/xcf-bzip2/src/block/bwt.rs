use std::mem;

pub(crate) fn inverse_bwt_checked(
    tt: &mut [u32],
    orig_ptr: usize,
    mut c: [u32; 256],
    poll: &mut dyn FnMut() -> Result<(), super::BlockError>,
) -> Result<u32, super::BlockError> {
    let mut sum = 0u32;

    for ci in c.iter_mut() {
        sum += mem::replace(ci, sum);
    }

    for i in 0..tt.len() {
        if i.is_multiple_of(4096) {
            poll()?;
        }
        let b = (tt[i] & 0xff) as usize;
        tt[c[b] as usize] |= (i as u32) << 8;
        c[b] += 1;
    }

    Ok(tt[orig_ptr] >> 8)
}
