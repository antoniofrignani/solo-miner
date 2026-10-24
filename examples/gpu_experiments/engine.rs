use anyhow::{Context, Result, anyhow, ensure};
use bitcoin::hashes::{Hash, HashEngine, sha256};
use objc2::{
    rc::{Retained, autoreleasepool},
    runtime::ProtocolObject,
};
use objc2_foundation::NSString;
use objc2_metal::*;
use serde::Serialize;
use std::{
    cell::{Cell, RefCell},
    ptr::NonNull,
    time::Instant,
};

const CAPACITY: usize = 1024;
const SOURCE: &str = include_str!("baseline.metal");
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kernel {
    Original,
    TwoSequential,
    TwoInterleaved,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grid {
    Groups,
    Threads,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Variant {
    pub name: &'static str,
    pub kernel: Kernel,
    pub grid: Grid,
    pub depth: usize,
    pub split: bool,
}
pub const VARIANTS: [Variant; 9] = [
    Variant {
        name: "sync",
        kernel: Kernel::Original,
        grid: Grid::Groups,
        depth: 1,
        split: false,
    },
    Variant {
        name: "double",
        kernel: Kernel::Original,
        grid: Grid::Groups,
        depth: 2,
        split: false,
    },
    Variant {
        name: "double_split",
        kernel: Kernel::Original,
        grid: Grid::Groups,
        depth: 2,
        split: true,
    },
    Variant {
        name: "two_sequential",
        kernel: Kernel::TwoSequential,
        grid: Grid::Groups,
        depth: 1,
        split: false,
    },
    Variant {
        name: "two_interleaved",
        kernel: Kernel::TwoInterleaved,
        grid: Grid::Groups,
        depth: 1,
        split: false,
    },
    Variant {
        name: "dispatch_threads",
        kernel: Kernel::Original,
        grid: Grid::Threads,
        depth: 1,
        split: false,
    },
    Variant {
        name: "double_threads",
        kernel: Kernel::Original,
        grid: Grid::Threads,
        depth: 2,
        split: false,
    },
    Variant {
        name: "double_two_sequential",
        kernel: Kernel::TwoSequential,
        grid: Grid::Groups,
        depth: 2,
        split: false,
    },
    Variant {
        name: "double_two_interleaved",
        kernel: Kernel::TwoInterleaved,
        grid: Grid::Groups,
        depth: 2,
        split: false,
    },
];
#[derive(Clone, Copy)]
pub struct Work {
    pub header: [u8; 80],
    pub target: [u8; 32],
    pub start: u32,
    pub count: u32,
}
struct Prepared {
    key: [u8; 76],
    words: [u32; 11],
}
pub struct Engine {
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    outputs: [Retained<ProtocolObject<dyn MTLBuffer>>; 2],
    busy: [Cell<bool>; 2],
    prepared: RefCell<Option<Prepared>>,
    pub variant: Variant,
    pub name: String,
    pub width: usize,
    pub max_threads: usize,
}
pub struct Output {
    pub work: Work,
    pub raw: Vec<u32>,
    pub gpu_start: f64,
    pub gpu_end: f64,
    pub wait_seconds: f64,
    pub latency_seconds: f64,
}
pub struct Pending<'a> {
    owner: &'a Engine,
    command: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    slot: usize,
    work: Work,
    diagnostic: bool,
    submitted: Instant,
    done: bool,
}
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.command.waitUntilCompleted();
            self.owner.busy[self.slot].set(false);
        }
    }
}
impl Pending<'_> {
    pub fn finish(mut self) -> Result<Output> {
        let wait = Instant::now();
        self.command.waitUntilCompleted();
        let wait_seconds = wait.elapsed().as_secs_f64();
        ensure!(
            self.command.status() == MTLCommandBufferStatus::Completed,
            "GPU error: {:?}",
            self.command.error()
        );
        // Only this completed command owns the slot until finish releases it.
        let ptr = self.owner.outputs[self.slot]
            .contents()
            .as_ptr()
            .cast::<u32>();
        let len = if self.diagnostic {
            self.work.count as usize * 8
        } else {
            (unsafe { ptr.read() } as usize).min(CAPACITY) + 1
        };
        let raw = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
        let output = Output {
            work: self.work,
            raw,
            gpu_start: self.command.GPUStartTime(),
            gpu_end: self.command.GPUEndTime(),
            wait_seconds,
            latency_seconds: self.submitted.elapsed().as_secs_f64(),
        };
        self.owner.busy[self.slot].set(false);
        self.done = true;
        Ok(output)
    }
}
impl Engine {
    pub fn new(variant: Variant) -> Result<Self> {
        let device = MTLCreateSystemDefaultDevice().context("No Metal GPU")?;
        ensure!(
            matches!(variant.grid, Grid::Groups) || device.supportsFamily(MTLGPUFamily::Apple4),
            "Nonuniform grids unsupported"
        );
        let source = if matches!(variant.kernel, Kernel::Original) {
            SOURCE.into()
        } else {
            format!("{SOURCE}\n{}", include_str!("two_nonce.metal"))
        };
        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(&source), None)
            .map_err(|e| anyhow!("Shader: {e}"))?;
        let function = library
            .newFunctionWithName(&NSString::from_str(match variant.kernel {
                Kernel::Original => "sha256d_scan",
                Kernel::TwoSequential => "sha256d_two_sequential",
                Kernel::TwoInterleaved => "sha256d_two_interleaved",
            }))
            .context("Kernel missing")?;
        let pipeline = device
            .newComputePipelineStateWithFunction_error(&function)
            .map_err(|e| anyhow!("Pipeline: {e}"))?;
        let max_threads = pipeline.maxTotalThreadsPerThreadgroup();
        let width = pipeline.threadExecutionWidth().min(max_threads);
        let buffer = || {
            device
                .newBufferWithLength_options(
                    (CAPACITY + 1) * 4,
                    MTLResourceOptions::StorageModeShared,
                )
                .context("Output allocation failed")
        };
        Ok(Self {
            queue: device.newCommandQueue().context("Queue missing")?,
            pipeline,
            outputs: [buffer()?, buffer()?],
            busy: [Cell::new(false), Cell::new(false)],
            prepared: RefCell::new(None),
            variant,
            name: device.name().to_string(),
            width,
            max_threads,
        })
    }
    fn prepare(&self, header: &[u8; 80]) -> [u32; 11] {
        let mut cached = self.prepared.borrow_mut();
        if cached.as_ref().is_none_or(|p| p.key != header[..76]) {
            let mut engine = sha256::Hash::engine();
            engine.input(&header[..64]);
            let bytes = engine.midstate().to_byte_array();
            let mut words = [0; 11];
            for (out, bytes) in words[..8].iter_mut().zip(bytes.as_chunks::<4>().0) {
                *out = u32::from_be_bytes(*bytes);
            }
            for (out, bytes) in words[8..].iter_mut().zip(header[64..76].as_chunks::<4>().0) {
                *out = u32::from_be_bytes(*bytes);
            }
            *cached = Some(Prepared {
                key: header[..76].try_into().unwrap(),
                words,
            });
        }
        cached.as_ref().unwrap().words
    }
    pub fn submit(&self, slot: usize, work: Work, diagnostic: bool) -> Result<Pending<'_>> {
        ensure!(
            slot < 2 && !self.busy[slot].get(),
            "Output slot is still in flight"
        );
        ensure!(
            work.count > 0
                && work.count <= 1_048_576
                && work.start.checked_add(work.count - 1).is_some()
                && (!diagnostic || work.count <= 128),
            "Invalid nonce range"
        );
        autoreleasepool(|_| {
            let command = self.queue.commandBuffer().context("Command missing")?;
            let encoder = command.computeCommandEncoder().context("Encoder missing")?;
            let params = [
                work.start,
                work.count,
                u32::from(diagnostic),
                CAPACITY as u32,
            ];
            let prepared = self.prepare(&work.header);
            unsafe {
                self.outputs[slot]
                    .contents()
                    .as_ptr()
                    .cast::<u32>()
                    .write(0);
                encoder.setBytes_length_atIndex(
                    NonNull::new(prepared.as_ptr().cast_mut().cast()).unwrap(),
                    std::mem::size_of_val(&prepared),
                    0,
                );
                encoder.setBytes_length_atIndex(
                    NonNull::new(work.target.as_ptr().cast_mut().cast()).unwrap(),
                    32,
                    1,
                );
                encoder.setBytes_length_atIndex(
                    NonNull::new(params.as_ptr().cast_mut().cast()).unwrap(),
                    std::mem::size_of_val(&params),
                    2,
                );
                encoder.setBuffer_offset_atIndex(Some(&self.outputs[slot]), 0, 3);
            }
            encoder.setComputePipelineState(&self.pipeline);
            let lanes = if matches!(self.variant.kernel, Kernel::Original) {
                1
            } else {
                2
            };
            let threads = (work.count as usize).div_ceil(lanes);
            let group = MTLSize {
                width: self.width,
                height: 1,
                depth: 1,
            };
            match self.variant.grid {
                Grid::Groups => encoder.dispatchThreadgroups_threadsPerThreadgroup(
                    MTLSize {
                        width: threads.div_ceil(self.width),
                        height: 1,
                        depth: 1,
                    },
                    group,
                ),
                Grid::Threads => encoder.dispatchThreads_threadsPerThreadgroup(
                    MTLSize {
                        width: threads,
                        height: 1,
                        depth: 1,
                    },
                    group,
                ),
            }
            encoder.endEncoding();
            self.busy[slot].set(true);
            let pending = Pending {
                owner: self,
                command,
                slot,
                work,
                diagnostic,
                submitted: Instant::now(),
                done: false,
            };
            pending.command.commit();
            Ok(pending)
        })
    }
    pub fn scan(&self, work: Work) -> Result<Vec<u32>> {
        let out = self.submit(0, work, false)?.finish()?;
        if out.raw[0] as usize > CAPACITY {
            let half = work.count / 2;
            let mut found = self.scan(Work {
                count: half,
                ..work
            })?;
            found.extend(self.scan(Work {
                start: work.start + half,
                count: work.count - half,
                ..work
            })?);
            return Ok(found);
        }
        let mut found = out.raw[1..].to_vec();
        found.sort_unstable();
        Ok(found)
    }
}
