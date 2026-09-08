//! Shared wgpu agreement-test harness.

use std::{
    future::Future,
    sync::{Arc, mpsc},
    task::{Context, Poll, Wake, Waker},
    thread,
};

use bytemuck::Pod;

pub fn request_device(label: &str) -> Option<(wgpu::AdapterInfo, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = match block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    })) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("SKIPPED: no compatible GPU adapter exists: {error}");
            return None;
        }
    };
    let adapter_info = adapter.get_info();
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
    }))
    .expect("a compatible adapter must provide a baseline compute device");
    Some((adapter_info, device, queue))
}

/// Compiles and runs one single-bind-group compute contract whose entry point
/// is `main` and whose workgroup size is 64 invocations along the x axis.
pub fn run_compute(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    source: &str,
    resources: &[wgpu::BindingResource<'_>],
    invocation_count: u32,
) {
    validate_wgsl(label, source);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let entries: Vec<_> = resources
        .iter()
        .enumerate()
        .map(|(binding, resource)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: resource.clone(),
        })
        .collect();
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(invocation_count.div_ceil(64), 1, 1);
    }
    queue.submit([encoder.finish()]);
}

/// Parses and validates assembled WGSL without requiring a GPU device.
pub fn validate_wgsl(label: &str, source: &str) {
    let module = wgpu::naga::front::wgsl::parse_str(source).unwrap_or_else(|error| {
        panic!(
            "shader '{label}' parsing error: {}",
            error.emit_to_string(source)
        )
    });
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|error| {
        panic!(
            "shader '{label}' validation error: {}",
            error.emit_to_string(source)
        )
    });
}

pub fn storage_output_buffer<T>(device: &wgpu::Device, label: &str, count: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (count * size_of::<T>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub fn readback<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    count: usize,
) -> Vec<T> {
    let size = (count * size_of::<T>()) as u64;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("agreement test readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("agreement test readback encoder"),
    });
    encoder.copy_buffer_to_buffer(source, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).unwrap();
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("GPU polling failed");
    receiver
        .recv()
        .unwrap()
        .expect("GPU readback mapping failed");
    let mapped = slice.get_mapped_range();
    let values = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    values
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}
