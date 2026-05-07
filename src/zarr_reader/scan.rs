use duckdb::core::FlatVector;

use super::types::{FillSentinel, ZarrDtype};

/// Copy one scalar element from raw little-endian bytes into a DuckDB vector slot,
/// applying NULL masking via the sentinel.
pub fn fill_scalar_element_pub(
    vector: &mut FlatVector<'_>,
    bytes: &[u8],
    dtype: &ZarrDtype,
    sentinel: &Option<FillSentinel>,
    src_idx: usize,
    _elem_size: usize,
    dst_idx: usize,
) {
    match dtype {
        ZarrDtype::Bool => {
            let val = bytes[src_idx] != 0;
            let slot = vector.as_mut_ptr::<bool>();
            unsafe { *slot.add(dst_idx) = val; }
        }
        ZarrDtype::Int8 => copy_scalar!(vector, bytes, i8, src_idx, dst_idx, sentinel),
        ZarrDtype::Int16 => copy_scalar!(vector, bytes, i16, src_idx, dst_idx, sentinel),
        ZarrDtype::Int32 => copy_scalar!(vector, bytes, i32, src_idx, dst_idx, sentinel),
        ZarrDtype::Int64 => copy_scalar!(vector, bytes, i64, src_idx, dst_idx, sentinel),
        ZarrDtype::UInt8 => copy_scalar!(vector, bytes, u8, src_idx, dst_idx, sentinel),
        ZarrDtype::UInt16 => copy_scalar!(vector, bytes, u16, src_idx, dst_idx, sentinel),
        ZarrDtype::UInt32 => copy_scalar!(vector, bytes, u32, src_idx, dst_idx, sentinel),
        ZarrDtype::UInt64 => copy_scalar!(vector, bytes, u64, src_idx, dst_idx, sentinel),
        ZarrDtype::Float32 => copy_scalar!(vector, bytes, f32, src_idx, dst_idx, sentinel),
        ZarrDtype::Float64 => copy_scalar!(vector, bytes, f64, src_idx, dst_idx, sentinel),
    }
}

/// Read any integer dtype from raw bytes at `start` as i64 (for packed-int decoding).
pub fn read_int_as_i64_pub(bytes: &[u8], dtype: &ZarrDtype, start: usize) -> i64 {
    match dtype {
        ZarrDtype::Int8 => bytes[start] as i8 as i64,
        ZarrDtype::Int16 => i16::from_le_bytes(bytes[start..start + 2].try_into().unwrap()) as i64,
        ZarrDtype::Int32 => i32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()) as i64,
        ZarrDtype::Int64 => i64::from_le_bytes(bytes[start..start + 8].try_into().unwrap()),
        ZarrDtype::UInt8 => bytes[start] as i64,
        ZarrDtype::UInt16 => u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap()) as i64,
        ZarrDtype::UInt32 => u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()) as i64,
        ZarrDtype::UInt64 => u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap()) as i64,
        _ => 0,
    }
}

// ── Null-check helpers ────────────────────────────────────────────────────────

macro_rules! copy_scalar {
    ($vector:expr, $bytes:expr, $T:ty, $src_idx:expr, $dst_idx:expr, $sentinel:expr) => {{
        let elem_size = std::mem::size_of::<$T>();
        let start = $src_idx * elem_size;
        let arr: [u8; std::mem::size_of::<$T>()] =
            $bytes[start..start + elem_size].try_into().unwrap();
        let val = <$T>::from_le_bytes(arr);
        if is_fill(val, $sentinel) {
            $vector.set_null($dst_idx);
        } else {
            let slot = $vector.as_mut_ptr::<$T>();
            unsafe { *slot.add($dst_idx) = val; }
        }
    }};
}

use copy_scalar;

fn is_fill<T: NullCheck>(val: T, sentinel: &Option<FillSentinel>) -> bool {
    val.check_fill(sentinel)
}

trait NullCheck: Copy {
    fn check_fill(self, sentinel: &Option<FillSentinel>) -> bool;
}

macro_rules! impl_null_check_int {
    ($T:ty, $Variant:ident, $cast:ty) => {
        impl NullCheck for $T {
            fn check_fill(self, s: &Option<FillSentinel>) -> bool {
                matches!(s, Some(FillSentinel::$Variant(v)) if *v == self as $cast)
            }
        }
    };
}

impl_null_check_int!(i8,  Int,  i64);
impl_null_check_int!(i16, Int,  i64);
impl_null_check_int!(i32, Int,  i64);
impl_null_check_int!(i64, Int,  i64);
impl_null_check_int!(u8,  UInt, u64);
impl_null_check_int!(u16, UInt, u64);
impl_null_check_int!(u32, UInt, u64);
impl_null_check_int!(u64, UInt, u64);

impl NullCheck for f32 {
    fn check_fill(self, s: &Option<FillSentinel>) -> bool {
        match s {
            Some(FillSentinel::Float(v)) => {
                if v.is_nan() { self.is_nan() } else { (self as f64 - v).abs() < f64::EPSILON }
            }
            _ => false,
        }
    }
}

impl NullCheck for f64 {
    fn check_fill(self, s: &Option<FillSentinel>) -> bool {
        match s {
            Some(FillSentinel::Float(v)) => {
                if v.is_nan() { self.is_nan() } else { (self - v).abs() < f64::EPSILON }
            }
            _ => false,
        }
    }
}
