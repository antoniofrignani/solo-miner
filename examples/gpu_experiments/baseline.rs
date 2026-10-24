use crate::{config::GpuProfile, pow};
use anyhow::{Context, Result, anyhow, ensure};
use bitcoin::hashes::{Hash, HashEngine, sha256};
use objc2::{
    rc::{Retained, autoreleasepool},
    runtime::ProtocolObject,
};
use objc2_foundation::NSString;
use objc2_metal::*;
use std::{cell::RefCell, ptr::NonNull};

const MAX_BATCH: u32 = 1_048_576;
const RESULT_CAPACITY: usize = 1024;
struct Prepared {
    key: [u8; 76],
    words: [u32; 11],
}
pub struct Gpu {
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    output: Retained<ProtocolObject<dyn MTLBuffer>>,
    pub name: String,
    profile: GpuProfile,
    prepared: RefCell<Option<Prepared>>,
}
impl Gpu {
    pub fn new() -> Result<Self> {
        Self::with_profile(GpuProfile::default())
    }
    pub fn with_profile(profile: GpuProfile) -> Result<Self> {
        let device = MTLCreateSystemDefaultDevice().context("No Metal GPU available")?;
        let name = device.name().to_string();
        let library = device
            .newLibraryWithSource_options_error(
                &NSString::from_str(include_str!("baseline.metal")),
                None,
            )
            .map_err(|e| anyhow!("Metal shader: {e}"))?;
        let function = library
            .newFunctionWithName(&NSString::from_str("sha256d_scan"))
            .context("Missing kernel")?;
        let pipeline = device
            .newComputePipelineStateWithFunction_error(&function)
            .map_err(|e| anyhow!("Metal pipeline: {e}"))?;
        let queue = device
            .newCommandQueue()
            .context("Metal command queue unavailable")?;
        let output = device
            .newBufferWithLength_options(
                (RESULT_CAPACITY + 1) * 4,
                MTLResourceOptions::StorageModeShared,
            )
            .context("GPU allocation failed")?;
        let gpu = Self {
            queue,
            pipeline,
            output,
            name,
            profile,
            prepared: RefCell::new(None),
        };
        gpu.self_test()?;
        Ok(gpu)
    }
    fn dispatch(
        &self,
        header: &[u8; 80],
        target: &[u8; 32],
        start: u32,
        count: u32,
        diagnostic: bool,
    ) -> Result<Vec<u32>> {
        ensure!(
            count > 0
                && count <= MAX_BATCH
                && (!diagnostic || count <= 128)
                && start.checked_add(count - 1).is_some(),
            "Invalid nonce range"
        );
        autoreleasepool(|_| {
            let command = self
                .queue
                .commandBuffer()
                .context("Command buffer unavailable")?;
            let encoder = command
                .computeCommandEncoder()
                .context("Compute encoder unavailable")?;
            encoder.setComputePipelineState(&self.pipeline);
            let params = [start, count, u32::from(diagnostic), RESULT_CAPACITY as u32];
            let prepared = self.prepare(header);
            // The previous command has completed before this shared buffer is reused.
            unsafe {
                self.output.contents().as_ptr().cast::<u32>().write(0);
            }
            // Metal copies setBytes immediately. Buffer stays owned and is read only after GPU completion.
            unsafe {
                encoder.setBytes_length_atIndex(
                    NonNull::new(prepared.as_ptr().cast_mut().cast()).unwrap(),
                    std::mem::size_of_val(&prepared),
                    0,
                );
                encoder.setBytes_length_atIndex(
                    NonNull::new(target.as_ptr().cast_mut().cast()).unwrap(),
                    32,
                    1,
                );
                encoder.setBytes_length_atIndex(
                    NonNull::new(params.as_ptr().cast_mut().cast()).unwrap(),
                    std::mem::size_of_val(&params),
                    2,
                );
                encoder.setBuffer_offset_atIndex(Some(&self.output), 0, 3);
            }
            let width = self
                .pipeline
                .threadExecutionWidth()
                .min(self.pipeline.maxTotalThreadsPerThreadgroup());
            encoder.dispatchThreadgroups_threadsPerThreadgroup(
                MTLSize {
                    width: (count as usize).div_ceil(width),
                    height: 1,
                    depth: 1,
                },
                MTLSize {
                    width,
                    height: 1,
                    depth: 1,
                },
            );
            encoder.endEncoding();
            command.commit();
            command.waitUntilCompleted();
            ensure!(
                command.status() == MTLCommandBufferStatus::Completed,
                "GPU error: {:?}",
                command.error()
            );
            let len = if diagnostic {
                count as usize * 8
            } else {
                let found =
                    unsafe { self.output.contents().as_ptr().cast::<u32>().read() } as usize;
                found.min(RESULT_CAPACITY) + 1
            };
            // Shared allocation is >= len*4 bytes, naturally aligned, and no command remains in flight.
            Ok(unsafe {
                std::slice::from_raw_parts(self.output.contents().as_ptr().cast::<u32>(), len)
            }
            .to_vec())
        })
    }
    pub fn scan(
        &self,
        header: &[u8; 80],
        target: &[u8; 32],
        start: u32,
        count: u32,
    ) -> Result<Vec<u32>> {
        let raw = self.dispatch(header, target, start, count, false)?;
        if raw[0] as usize > RESULT_CAPACITY {
            // Retry overflowing ranges in smaller batches; never drop valid candidates.
            let half = count / 2;
            let mut found = self.scan(header, target, start, half)?;
            found.extend(self.scan(header, target, start + half, count - half)?);
            return Ok(found);
        }
        let mut found = raw[1..].to_vec();
        found.sort_unstable();
        Ok(found)
    }
    pub fn batch_size(&self) -> u32 {
        self.profile.batch_size()
    }
    fn prepare(&self, header: &[u8; 80]) -> [u32; 11] {
        let mut cached = self.prepared.borrow_mut();
        if cached.as_ref().is_none_or(|p| p.key != header[..76]) {
            let mut engine = sha256::Hash::engine();
            engine.input(&header[..64]);
            let midstate = engine.midstate().to_byte_array();
            let mut words = [0; 11];
            for (word, bytes) in words[..8].iter_mut().zip(midstate.as_chunks::<4>().0) {
                *word = u32::from_be_bytes(*bytes);
            }
            for (word, bytes) in words[8..].iter_mut().zip(header[64..76].as_chunks::<4>().0) {
                *word = u32::from_be_bytes(*bytes);
            }
            *cached = Some(Prepared {
                key: header[..76].try_into().unwrap(),
                words,
            });
        }
        cached.as_ref().unwrap().words
    }
    fn self_test(&self) -> Result<()> {
        let mut header = pow::genesis_header();
        let start = u32::from_le_bytes(header[76..80].try_into().unwrap()) - 32;
        let raw = self.dispatch(&header, &[255; 32], start, 64, true)?;
        for (i, words) in raw.as_chunks::<8>().0.iter().enumerate() {
            header[76..80].copy_from_slice(&(start + i as u32).to_le_bytes());
            let actual: Vec<u8> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
            ensure!(actual == pow::hash(&header), "GPU SHA256d self-test failed");
        }
        let target = pow::compact_target(0x1d00ffff)?;
        ensure!(
            self.scan(&header, &target, start, 64)? == vec![start + 32],
            "GPU target self-test failed"
        );
        // Exercise all target words and arbitrary headers, not just a leading-zero target.
        for seed in [0u8, 73, 255] {
            header.fill(seed);
            let mut probe = header;
            probe[76..80].copy_from_slice(&7u32.to_le_bytes());
            let target = pow::hash(&probe);
            let expected: Vec<u32> = (0..32u32)
                .filter(|n| {
                    probe[76..80].copy_from_slice(&n.to_le_bytes());
                    pow::meets(&pow::hash(&probe), &target)
                })
                .collect();
            ensure!(
                self.scan(&header, &target, 0, 32)? == expected,
                "Target comparison self-test failed"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires a real Metal GPU; checks compact output, cache invalidation and nonce boundaries"]
    fn metal_profiles_match_cpu_and_preserve_overflow() -> Result<()> {
        for profile in GpuProfile::ALL {
            let gpu = Gpu::with_profile(profile)?;
            let mut header = pow::genesis_header();
            for seed in [0u8, 17, 255] {
                header[0] = seed;
                header[64] = seed;
                let mut probe = header;
                probe[76..80].copy_from_slice(&37u32.to_le_bytes());
                let target = pow::hash(&probe);
                let expected: Vec<u32> = (0..4097u32)
                    .filter(|nonce| {
                        probe[76..80].copy_from_slice(&nonce.to_le_bytes());
                        pow::meets(&pow::hash(&probe), &target)
                    })
                    .collect();
                assert_eq!(gpu.scan(&header, &target, 0, 4097)?, expected);
            }
            // A target accepting every hash overflows the compact candidate buffer.
            let start = u32::MAX - 2048;
            assert_eq!(
                gpu.scan(&header, &[255; 32], start, 2049)?,
                (start..=u32::MAX).collect::<Vec<_>>()
            );
            assert!(
                gpu.scan(&header, &[0; 32], 0, profile.batch_size())?
                    .is_empty()
            );
            assert!(gpu.scan(&header, &[0; 32], u32::MAX, 2).is_err());
            assert!(gpu.scan(&header, &[0; 32], 0, 0).is_err());
        }
        Ok(())
    }
}
