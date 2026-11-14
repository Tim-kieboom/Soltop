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
    };
}

newtype!(
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Bytes(u64) { as_u64 }
);
newtype!(
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Gb(f32) { as_f32 }
);
newtype!(
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Degree(f32) { as_f32 }
);
newtype!(
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Percent(f32) { as_f32 }
);

#[derive(Debug, Clone, Copy, Default)]
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

pub fn safe_div(a: f32, b: f32) -> f32 {
    if b == 0.0 { 0.0 } else { a / b }
}
