//! Checked logical buffer capacities; excludes allocator headers/RSS.
pub(crate) fn vector_bytes<T>(values: &alloc::vec::Vec<T>) -> Option<usize> {
    values.capacity().checked_mul(core::mem::size_of::<T>())
}
