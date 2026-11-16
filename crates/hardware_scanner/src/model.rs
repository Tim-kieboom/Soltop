use std::{ffi::OsStr, fmt::Debug, path::Path};

use nvml_wrapper::error::NvmlError;
use thiserror::Error;

macro_rules! newtype {
    ($(#[$meta:meta])* $vis:vis struct $name:ident($ty:ty) { $as:ident }) => {
        $(#[$meta])*
        $vis struct $name($ty);

        impl $name {
            #[allow(unused)]
            $vis fn new(v: $ty) -> Self {
                Self(v)
            }

            #[allow(unused)]
            $vis fn $as(&self) -> $ty {
                self.0
            }
        }

        impl AsF64 for $name {
            fn as_f64(&self) -> f64 {
                self.0 as f64
            }
        }
    };
}

newtype!(
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Bytes(u64) { as_u64 }
);
newtype!(
    #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
    pub struct Gb(f32) { as_f32 }
);
newtype!(
    #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
    pub struct Degree(f32) { as_f32 }
);
newtype!(
    #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
    pub struct Percent(f32) { as_f32 }
);

pub trait AsF64 {
    fn as_f64(&self) -> f64;
}

impl Bytes {
    pub fn to_gb(self, decimal: u8) -> Gb {
        let deci = 10_f32.powi(decimal as i32);
        let gb = ((self.0 as f32 / 1000000000.0) * deci).round() / deci;
        Gb(gb)
    }
}

pub type ScanResult<T = ()> = Result<T, ScanError>;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("sysinfo error: {0}")]
    Sysinfo(#[from] sysinfo::Error),
    #[error("gpu error: {0}")]
    Gpu(#[from] NvmlError),
}

#[derive(Debug, Default)]
pub struct CpuInfo {
    pub core_usage: Vec<Percent>,
    pub temperature: Option<Degree>,
    pub total_usage: Option<Percent>,
}

#[derive(Debug, Default)]
pub struct RamInfo {
    pub used: Option<Bytes>,
    pub total: Option<Bytes>,
    pub temperature: Option<Degree>,
}

#[derive(Debug, Default)]
pub struct GpuInfo {
    pub gpu: Option<Percent>,
    pub memory_usage: Option<Percent>,
    pub memory_total: Option<Bytes>,
    pub temperature: Option<Degree>,
}

#[derive(Debug)]
pub struct Process {
    pub pid: sysinfo::Pid,
    pub memory: Bytes,
    pub name: Box<Path>,
    pub cpu_usage: Percent,
    pub gpu_memory: Option<Bytes>,
    pub gpu_usage: Option<Percent>,
    pub gpu_number: Option<GpuNumbers>,
}
impl Process {
    pub fn file_name(&self) -> &OsStr {
        self.name.file_name().unwrap_or(self.name.as_os_str())
    }
}

#[derive(Clone, Copy, Default)]
pub struct GpuNumbers(u64);
impl GpuNumbers {
    pub fn from_index(i: usize) -> Self {
        // i expect no one insane eneugh to have 65 gpu's if so gpu 65 notes as gpu 64
        Self(1 << i.min(63))
    }

    pub fn from_raw_u64(v: u64) -> Self {
        Self(v)
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn has_overlap(&self, other: Self) -> bool {
        for i in other.iter() {
            if self.contains(i) {
                return true;
            }
        }

        false
    }

    pub fn contains(&self, index: usize) -> bool {
        self.0 & (1 << index.min(63)) != 0
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }

    pub fn iter(self) -> impl Iterator<Item = usize> {
        let mut value = self.0;
        std::iter::from_fn(move || {
            if value == 0 {
                None
            } else {
                let idx = value.trailing_zeros() as usize;
                value &= value - 1;
                Some(idx)
            }
        })
    }
}
impl Debug for GpuNumbers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

pub fn safe_div(a: f32, b: f32) -> f32 {
    if b == 0.0 { 0.0 } else { a / b }
}
