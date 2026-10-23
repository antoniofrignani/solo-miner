use crate::pow;
use anyhow::{Context, Result, anyhow, ensure};
use objc2::{
    rc::{Retained, autoreleasepool},
    runtime::ProtocolObject,
};
use objc2_foundation::NSString;
use objc2_metal::*;
use std::ptr::NonNull;

pub const BATCH: u32 = 65536;
pub struct Gpu {
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    output: Retained<ProtocolObject<dyn MTLBuffer>>,
    pub name: String,
}
impl Gpu {
    pub fn new() -> Result<Self> {
        let device = MTLCreateSystemDefaultDevice().context("No Metal GPU available")?;
        let name = device.name().to_string();
        let library = device
            .newLibraryWithSource_options_error(
                &NSString::from_str(include_str!("sha256.metal")),
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
            .newBufferWithLength_options(BATCH as usize * 32, MTLResourceOptions::StorageModeShared)
            .context("GPU allocation failed")?;
        let gpu = Self {
            queue,
            pipeline,
            output,
            name,
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
            count > 0 && count <= BATCH && start.checked_add(count - 1).is_some(),
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
            let params = [start, count, u32::from(diagnostic)];
            // Metal copies setBytes immediately. Buffer stays owned and is read only after GPU completion.
            unsafe {
                encoder.setBytes_length_atIndex(
                    NonNull::new(header.as_ptr().cast_mut().cast()).unwrap(),
                    80,
                    0,
                );
                encoder.setBytes_length_atIndex(
                    NonNull::new(target.as_ptr().cast_mut().cast()).unwrap(),
                    32,
                    1,
                );
                encoder.setBytes_length_atIndex(
                    NonNull::new(params.as_ptr().cast_mut().cast()).unwrap(),
                    12,
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
            let len = count as usize * if diagnostic { 8 } else { 1 };
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
        Ok(self
            .dispatch(header, target, start, count, false)?
            .into_iter()
            .enumerate()
            .filter_map(|(i, flag)| (flag != 0).then_some(start + i as u32))
            .collect())
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
