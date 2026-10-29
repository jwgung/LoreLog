//! One sampler per OS. Windows only for now; Linux and macOS come later (see docs in desktop/README.md).
#[cfg(not(windows))]
use life_agent_core::agent::Sampler;
#[cfg(not(windows))]
use life_agent_core::segments::Sample;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::WindowsSampler as PlatformSampler;

#[cfg(not(windows))]
pub struct PlatformSampler;

#[cfg(not(windows))]
impl Sampler for PlatformSampler {
    fn sample(&mut self) -> Result<Sample, String> {
        Err(format!(
            "{} is not supported yet (Windows only for now)",
            std::env::consts::OS
        ))
    }
}

pub fn new() -> PlatformSampler {
    #[cfg(windows)]
    return windows::WindowsSampler;
    #[cfg(not(windows))]
    return PlatformSampler;
}
