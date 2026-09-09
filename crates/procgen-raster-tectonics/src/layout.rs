//! Host mirrors of the structs the kernels declare.
//!
//! Each type here matches one struct in `wgsl/bindings.wgsl` byte for byte, so
//! the pipeline sizes its buffers and addresses their dispatch arguments and
//! diagnostics by field rather than by counting words.
//! [`tests::host_mirrors_match_the_kernels_own_layout`] holds the two sides
//! together by asking `naga` for the layout it computed.

use crate::evolution::RasterEvolutionConfig;
use crate::field::PLATE_ID_COUNT;
use crate::partition::{
    RasterPlatePartitionConfig, SEED_REDUCTION_WORKGROUPS, first_seed_cell, fold_growth_key,
};
use procgen_tectonics::BoundaryClass;
use std::mem::offset_of;

/// The uniform block every kernel reads. The trailing pair rounds the block to
/// the sixteen-byte stride a uniform binding requires.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PackedTectonicsConfig {
    resolution: u32,
    cell_count: u32,
    plate_count: u32,
    major_plate_count: u32,
    head_start_cost: u32,
    growth_roughness: u32,
    growth_key: u32,
    first_seed_cell: u32,
    target_ocean_fraction: f32,
    minimum_convergence: f32,
    padding: [u32; 2],
}

impl PackedTectonicsConfig {
    pub(crate) fn new(
        partition: &RasterPlatePartitionConfig,
        evolution: &RasterEvolutionConfig,
        resolution: u32,
        cell_count: u32,
    ) -> Self {
        Self {
            resolution,
            cell_count,
            plate_count: partition.plate_count(),
            major_plate_count: partition.major_plate_count,
            head_start_cost: partition.head_start_cost(resolution),
            growth_roughness: partition.growth_roughness,
            growth_key: fold_growth_key(partition.seed),
            first_seed_cell: first_seed_cell(partition.seed, cell_count),
            target_ocean_fraction: evolution.crust.target_ocean_fraction,
            minimum_convergence: evolution.migration.minimum_convergence,
            padding: [0; 2],
        }
    }
}

/// The counters the kernels accumulate, copied back once per run.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PackedDiagnostics {
    pub(crate) longest_relaxation: u32,
    pub(crate) total_area: u32,
    pub(crate) ocean_area: u32,
    pub(crate) empty_plate_count: u32,
    pub(crate) migrated_cell_count: u32,
    pub(crate) boundary_class_counts: [u32; BoundaryClass::ALL.len()],
}

/// Mirror of `RasterTectonicsState`. Nothing reads its fields; `size_of` and
/// `offset_of!` are its whole purpose.
#[repr(C)]
pub(crate) struct PackedTectonicsState {
    pub(crate) relax_dispatch: [u32; 3],
    pass_index: u32,
    phase_passes: u32,
    next_plate: u32,
    chosen_cell: u32,
    frontier_count: [u32; 2],
    pub(crate) diagnostics: PackedDiagnostics,
    plate_areas: [u32; PLATE_ID_COUNT as usize],
    seed_partials: [[u32; 2]; SEED_REDUCTION_WORKGROUPS as usize],
}

const _: () = assert!(
    offset_of!(PackedTectonicsState, seed_partials) % 8 == 0,
    "WGSL aligns the seed candidates to eight bytes, so the fields before them \
     must occupy a multiple of eight"
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::PipelineTuning;
    use crate::field::RasterPlate;
    use crate::kernels::tectonics_kernel_source;

    /// Returns the WGSL member offsets and size of one kernel struct.
    fn wgsl_layout(name: &str) -> (Vec<(String, u32)>, u32) {
        let source = tectonics_kernel_source(PipelineTuning::default());
        let module =
            wgpu::naga::front::wgsl::parse_str(&source).expect("the assembled kernels must parse");
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("the kernels must declare {name}"));
        let wgpu::naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("{name} must be a struct");
        };
        (
            members
                .iter()
                .map(|member| {
                    (
                        member.name.clone().expect("kernel members are named"),
                        member.offset,
                    )
                })
                .collect(),
            *span,
        )
    }

    #[test]
    fn host_mirrors_match_the_kernels_own_layout() {
        let (members, span) = wgsl_layout("RasterTectonicsState");
        assert_eq!(span as usize, size_of::<PackedTectonicsState>());
        assert_eq!(
            members,
            [
                (
                    "relax_dispatch_x",
                    offset_of!(PackedTectonicsState, relax_dispatch)
                ),
                (
                    "relax_dispatch_y",
                    offset_of!(PackedTectonicsState, relax_dispatch) + 4
                ),
                (
                    "relax_dispatch_z",
                    offset_of!(PackedTectonicsState, relax_dispatch) + 8
                ),
                ("pass_index", offset_of!(PackedTectonicsState, pass_index)),
                (
                    "phase_passes",
                    offset_of!(PackedTectonicsState, phase_passes)
                ),
                ("next_plate", offset_of!(PackedTectonicsState, next_plate)),
                ("chosen_cell", offset_of!(PackedTectonicsState, chosen_cell)),
                (
                    "frontier_count",
                    offset_of!(PackedTectonicsState, frontier_count)
                ),
                ("diagnostics", offset_of!(PackedTectonicsState, diagnostics)),
                ("plate_areas", offset_of!(PackedTectonicsState, plate_areas)),
                (
                    "seed_partials",
                    offset_of!(PackedTectonicsState, seed_partials)
                ),
            ]
            .map(|(name, offset)| (name.to_owned(), offset as u32))
        );

        let (_, span) = wgsl_layout("RasterTectonicsDiagnostics");
        assert_eq!(span as usize, size_of::<PackedDiagnostics>());
        let (_, span) = wgsl_layout("RasterTectonicsConfig");
        assert_eq!(span as usize, size_of::<PackedTectonicsConfig>());
        let (members, span) = wgsl_layout("RasterPlate");
        assert_eq!(span as usize, size_of::<RasterPlate>());
        assert_eq!(
            members.last().map(|(_, offset)| *offset as usize),
            Some(offset_of!(RasterPlate, angular_velocity))
        );
    }
}
