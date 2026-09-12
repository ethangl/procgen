//! What a run counts while it runs, and nothing else.
//!
//! These are totals rather than history: a run sums an event count per step
//! and keeps no record of which step it came from. They sit beside the run's
//! config and its errors rather than inside it, because the viewer reads them
//! and `evolution.rs` is the run itself.

use crate::{lifecycle::LifecycleEvents, transport::TransportCounts};

/// Totals accumulated without retaining per-step history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlateEvolutionDiagnostics {
    /// Steps that changed ownership or made ocean floor.
    pub active_step_count: usize,
    /// Cells whose owner changed, summed over steps. A cell can change more
    /// than once.
    pub owner_change_count: usize,
    /// Particles a trench destroyed across all steps.
    pub subducted_particle_count: usize,
    /// Particles made in cells no material reached across all steps. This is
    /// the only place a run creates material.
    pub born_particle_count: usize,
    /// Cells holding material of more than one plate after subduction,
    /// summed over steps. A collision stacks rather than destroys, and this
    /// counts only that: two parcels of one plate are lattice noise that
    /// spreads back out next step.
    pub collided_cell_count: usize,
    /// Deepest such column over all steps, the foreign material plus the
    /// parcel that won the cell. Zero for a run in which nothing collided.
    pub maximum_collision_stack: usize,
    /// Cells that held no particle and read one nearby, summed over steps.
    pub sampled_cell_count: usize,
    /// Parcels a collision merged into the continent already there, summed
    /// over steps, plus the orphans compaction merged. It is the count of
    /// events; what they made is thickness.
    pub accreted_particle_count: usize,
    /// Parcels the flow pass moved from a column to a thinner neighbour,
    /// summed over steps. It is what turns a spike into a plateau.
    pub thickness_transfer_count: usize,

    /// Original parcels of continent before step zero and after the last
    /// step, summed over every column that holds any. The two are equal for
    /// every run: continental material is neither created nor destroyed, and
    /// a collision merges two parcels into one column rather than losing
    /// one. The continental *cell* count is a raster of that material and is
    /// not equal.
    pub starting_continental_thickness: u32,
    pub final_continental_thickness: u32,
    /// The deepest continental column the run ends with, in parcels.
    pub maximum_thickness: u32,
    /// Cells standing on more than one parcel, which is the extent of the
    /// collision plateaus the run built.
    pub thickened_cell_count: usize,

    /// Continental particles the run ends with that no cell reads, because
    /// another parcel of continent is in the cell with them. It is the gap
    /// between the material a run conserves and the continental cells that
    /// material accounts for, and nothing ever spreads it back out.
    pub covered_continental_particle_count: usize,
    /// The subset of those that lie under a cell their own plate does not
    /// own, which is what a collision stacked rather than what one plate's
    /// own material crowded together.
    pub foreign_continental_particle_count: usize,
    /// Continental plates that split in two across all steps.
    pub rift_count: usize,
    /// Rift arcs that failed to separate a plate into two pieces, which left
    /// the plate exactly as it was.
    pub failed_rift_count: usize,
    /// Continental pairs that merged across all steps.
    pub suture_count: usize,
}

impl PlateEvolutionDiagnostics {
    /// Records one step's transport and returns whether it changed anything.
    pub(crate) fn record_transport(&mut self, counts: TransportCounts) -> bool {
        self.owner_change_count += counts.owner_change_count;
        self.subducted_particle_count += counts.subducted_particle_count;
        self.born_particle_count += counts.born_particle_count;
        self.accreted_particle_count += counts.accreted_particle_count;
        self.thickness_transfer_count += counts.thickness_transfer_count;

        self.collided_cell_count += counts.collided_cell_count;
        self.maximum_collision_stack = self
            .maximum_collision_stack
            .max(counts.maximum_collision_stack);
        self.sampled_cell_count += counts.sampled_cell_count;
        counts.owner_change_count > 0 || counts.born_particle_count > 0
    }

    pub(crate) fn record_lifecycle(&mut self, events: LifecycleEvents) {
        self.rift_count += events.rift_count;
        self.failed_rift_count += events.failed_rift_count;
        self.suture_count += events.suture_count;
    }
}
