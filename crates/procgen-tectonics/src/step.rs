//! One evolution step: the state it advances and the three moves it makes.
//!
//! A step raises deformation along the boundaries it starts from, moves
//! ownership across the edges whose closing debt has paid for a cell, and
//! pulls what the cells carry one cell upstream. The first happens before the
//! other two because uplift happens at the boundary and the material moves
//! afterwards.
//!
//! Two per-cell fields travel with the crust rather than being recomputed from
//! the current state: the step a cell's crust was created, and the deformation
//! the boundaries have raised on it. Both move under one rule, so
//! [`CarriedFields`] holds them together and one decision per cell applies
//! to both: migration moves what the advancing cell carries onto the cell it
//! overrides, advection moves what the upstream neighbour carries, and new
//! crust starts flat and born now.

use crate::{
    BoundaryClass, BoundaryClassification, CellCrust, CrustClassification, PlateEvolutionConfig,
    PlateKinematics, PlateMigration, PlateMigrationError, PlatePartition,
    deformation::accumulate_boundary_deformation, migrate_plates_once,
    migration::accumulate_closing_distances,
};
use procgen_sphere_mesh::SphereMesh;

/// Everything a cell carries across a step, held column by column so that
/// consumers can borrow the birth field on its own.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CarriedFields {
    /// Step at which each cell's crust was created; `None` is original
    /// continental crust that evolution never re-made.
    pub(crate) birth: Vec<Option<i32>>,
    /// Signed deformation accumulated on each cell at the boundaries current
    /// in each step it has lived through.
    pub(crate) deformation: Vec<f32>,
}

impl CarriedFields {
    /// The fields a run starts from: the birth prior, and crust nothing has
    /// deformed yet.
    pub(crate) fn new(birth: Vec<Option<i32>>) -> Self {
        Self {
            deformation: vec![0.0; birth.len()],
            birth,
        }
    }

    /// Moves everything `from` carried in `previous` onto `to`. Migration and
    /// advection are this one move; they differ only in which cell they read.
    fn move_onto(&mut self, previous: &Self, to: usize, from: usize) {
        self.birth[to] = previous.birth[from];
        self.deformation[to] = previous.deformation[from];
    }

    /// Replaces a cell with crust made this step: new crust is flat.
    fn reborn(&mut self, cell: usize, step: i32) {
        self.birth[cell] = Some(step);
        self.deformation[cell] = 0.0;
    }
}

/// The state one step advances, together with the inputs every step reads.
///
/// Ownership, what the cells carry, and the two debts move together once per
/// step and nothing outside evolution owns a step, so holding them in one
/// place keeps the per-step solves in `migration.rs` pure functions of the
/// current state rather than functions that also return several vectors.
pub(crate) struct EvolvingWorld<'a> {
    pub(crate) mesh: &'a SphereMesh,
    pub(crate) crust: &'a CrustClassification,
    pub(crate) kinematics: &'a PlateKinematics,
    pub(crate) config: PlateEvolutionConfig,
    /// The one distance every accumulated displacement is measured against.
    pub(crate) cell_width: f32,
    pub(crate) partition: PlatePartition,
    pub(crate) carried: CarriedFields,
    /// Closing distance accumulated per boundary edge, in model units.
    pub(crate) edge_closing: Vec<f32>,
    /// Distance travelled per cell since its last pull, in model units.
    pub(crate) cell_travel: Vec<f32>,
}

impl EvolvingWorld<'_> {
    /// Raises this step's share of the boundary profiles onto the cells the
    /// current boundaries run through, and returns how many cells sourced one.
    ///
    /// A step spends `step_duration` of the full deformation time, so a
    /// boundary that stays saturated for that whole time reaches the full
    /// profile offset and one that passes through leaves a fraction of it.
    pub(crate) fn deform(&mut self, boundaries: &BoundaryClassification) -> usize {
        let config = self.config.deformation;
        accumulate_boundary_deformation(
            self.mesh,
            &self.partition,
            CellCrust {
                cell_birth: &self.carried.birth,
            },
            boundaries,
            &config,
            self.config.step_duration / config.full_deformation_time,
            &mut self.carried.deformation,
        )
    }

    /// Advances every edge's closing debt and applies the migrations the debts
    /// have paid for. A migrating cell takes everything the advancing cell
    /// carries across the edge, and the winning edge's debt drops by one cell
    /// width.
    pub(crate) fn migrate(
        &mut self,
        boundaries: &BoundaryClassification,
    ) -> Result<PlateMigration, PlateMigrationError> {
        accumulate_closing_distances(
            boundaries,
            &mut self.edge_closing,
            self.config.migration,
            self.config.step_duration,
        );
        let migration = migrate_plates_once(
            self.mesh,
            &self.partition,
            self.crust,
            boundaries,
            &self.edge_closing,
            self.cell_width,
        )?;

        let previous = self.carried.clone();
        for (cell, change) in migration.cell_changes.iter().enumerate() {
            let Some(change) = change else { continue };
            let cells = self.mesh.edges[change.boundary_edge].cells;
            let advancing = if cells[0] == cell { cells[1] } else { cells[0] };
            self.carried.move_onto(&previous, cell, advancing);
            self.edge_closing[change.boundary_edge] -= self.cell_width;
        }
        self.partition.clone_from(&migration.partition);
        Ok(migration)
    }

    /// Advances every cell's travel debt and pulls what the cells that have
    /// paid for it carry upstream by one cell.
    ///
    /// A cell's upstream neighbour is the same-plate neighbour lying most
    /// nearly opposite its velocity. A cell with none is at the plate's
    /// trailing edge: if the boundary behind it is a ridge it is new crust
    /// born this step, flat because nothing has deformed it yet, and otherwise
    /// it keeps what it has. Every pull reads the fields as they stood before
    /// this substep, so the update is simultaneous.
    pub(crate) fn advect(&mut self, boundaries: &BoundaryClassification, step: i32) -> usize {
        let mesh = self.mesh;
        let previous = self.carried.clone();
        let mut born_cell_count = 0;

        for cell in 0..mesh.cell_count() {
            let plate = self.partition.cell_plates[cell];
            let center = mesh.cell_centers[cell];
            let velocity = self.kinematics.velocity_at(plate, center);
            self.cell_travel[cell] += velocity.length() * self.config.step_duration;
            if self.cell_travel[cell] < self.cell_width {
                continue;
            }
            self.cell_travel[cell] -= self.cell_width;

            let mut upstream: Option<(f32, usize)> = None;
            let mut behind: Option<(f32, usize)> = None;
            for corner in mesh.cell_corners(cell) {
                let opposition = (mesh.cell_centers[corner.neighbor] - center).dot(-velocity);
                if opposition <= 0.0 {
                    continue;
                }
                if behind.is_none_or(|(best, _)| opposition > best) {
                    behind = Some((opposition, corner.edge));
                }
                if self.partition.cell_plates[corner.neighbor] == plate
                    && upstream.is_none_or(|(best, _)| opposition > best)
                {
                    upstream = Some((opposition, corner.neighbor));
                }
            }

            match upstream {
                Some((_, neighbor)) => self.carried.move_onto(&previous, cell, neighbor),
                None => {
                    let opening = behind.is_some_and(|(_, edge)| {
                        boundaries.edge_classes[edge] == BoundaryClass::Divergent
                    });
                    if opening {
                        self.carried.reborn(cell, step);
                        born_cell_count += 1;
                    }
                }
            }
        }
        born_cell_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CrustClass;
    use crate::field::mean_cell_width;
    use crate::test_support::{rift_config, two_plate_fixture};

    /// Birth out of every cell's reach, so a cell carrying it was reborn rather
    /// than handed it by a neighbour.
    const REBIRTH_STEP: i32 = 1_000;

    #[test]
    fn advection_pulls_deformation_from_the_cell_it_pulls_birth_from() {
        // Driven a substep at a time: the rule is about what one advection does
        // to one cell, which a whole run's boundaries would bury.
        let fixture = two_plate_fixture(1.0, vec![CrustClass::Continental; 2]);
        let mut world = EvolvingWorld {
            mesh: &fixture.mesh,
            crust: &fixture.crust,
            kinematics: &fixture.kinematics,
            config: rift_config(1),
            cell_width: mean_cell_width(&fixture.mesh),
            partition: fixture.partition.clone(),
            carried: CarriedFields::new(fixture.birth_prior.cell_birth.clone()),
            edge_closing: vec![0.0; fixture.mesh.edge_count()],
            cell_travel: vec![0.0; fixture.mesh.cell_count()],
        };
        // Stamp each cell with its own index in both fields, so where a value
        // ends up names the cell it came from.
        for cell in 0..fixture.mesh.cell_count() {
            world.carried.birth[cell] = Some(cell as i32);
            world.carried.deformation[cell] = cell as f32;
        }

        let born_cell_count = world.advect(&fixture.boundaries, REBIRTH_STEP);
        assert!(born_cell_count > 0, "the fixture must rift somewhere");
        let mut pulled_cell_count = 0;
        for cell in 0..fixture.mesh.cell_count() {
            match world.carried.birth[cell].unwrap() {
                REBIRTH_STEP => assert_eq!(
                    world.carried.deformation[cell], 0.0,
                    "cell {cell} is new crust and nothing has deformed it"
                ),
                source => {
                    pulled_cell_count += usize::from(source as usize != cell);
                    assert_eq!(
                        world.carried.deformation[cell], source as f32,
                        "cell {cell} took its deformation from a different cell than its birth"
                    );
                }
            }
        }
        assert!(pulled_cell_count > 0, "the fixture must advect somewhere");
    }
}
