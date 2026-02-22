use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

/// Resource consumption profile for a rate-limited process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceProfile {
    /// Number of CPU cores this process consumes.
    pub cpus: u32,
    /// Memory this process consumes in GiB.
    pub mem_gb: u32,
}

impl ResourceProfile {
    pub const fn new(cpus: u32, mem_gb: u32) -> Self {
        Self { cpus, mem_gb }
    }

    /// Returns true if the provided available resources can satisfy this profile's requirements.
    pub fn has_free_resources(&self, available: &ResourceProfile) -> bool {
        self.cpus <= available.cpus && self.mem_gb <= available.mem_gb
    }
}

impl fmt::Display for ResourceProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} CPUs, {} GiB", self.cpus, self.mem_gb)
    }
}

impl Add for ResourceProfile {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            cpus: self.cpus + other.cpus,
            mem_gb: self.mem_gb + other.mem_gb,
        }
    }
}

impl AddAssign for ResourceProfile {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl Sub for ResourceProfile {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self {
            cpus: self.cpus.saturating_sub(other.cpus),
            mem_gb: self.mem_gb.saturating_sub(other.mem_gb),
        }
    }
}

impl SubAssign for ResourceProfile {
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}
