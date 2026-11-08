use super::elements::{COVALENT_RADII, SYMBOLS};
use crate::document::Document;
use glam::{Mat4, Quat, Vec3};

#[derive(Clone)]
pub struct DisplayAtom {
    pub position: Vec3,
    pub radius: f32,
    pub color: [f32; 4],
    pub atomic_number: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayBond {
    pub atoms: [usize; 2],
    pub order: u8,
}
#[derive(Default)]
pub struct Scene {
    pub atoms: Vec<DisplayAtom>,
    pub bonds: Vec<DisplayBond>,
    pub center: Vec3,
    pub radius: f32,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    pub model: [[f32; 4]; 4],
    pub normal: [[f32; 4]; 4],
    pub color: [f32; 4],
}
impl Instance {
    fn new(model: Mat4, color: [f32; 4]) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            normal: model.inverse().transpose().to_cols_array_2d(),
            color,
        }
    }
}
impl Scene {
    pub fn from_document(document: Option<&Document>) -> Self {
        let Some(document) = document else {
            return Self::default();
        };
        let atoms: Vec<_> = document
            .atoms
            .iter()
            .map(|atom| {
                let z = atom.atomic_number as usize;
                DisplayAtom {
                    position: Vec3::from_array(atom.position_angstrom.map(|v| v as f32)),
                    radius: (COVALENT_RADII[z] * 0.38 + 0.12).clamp(0.22, 0.8),
                    color: color(atom.atomic_number),
                    atomic_number: atom.atomic_number,
                }
            })
            .collect();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for atom in &atoms {
            min = min.min(atom.position);
            max = max.max(atom.position);
        }
        let center = (min + max) * 0.5;
        let radius = atoms
            .iter()
            .map(|a| a.position.distance(center) + a.radius)
            .fold(0.5, f32::max);
        let mut bonds = Vec::new();
        for (i, a) in atoms.iter().enumerate() {
            for (j, b) in atoms[..i].iter().enumerate() {
                // Geometry-only hints: no bond order, charge/valence model or chemistry mutation.
                // Noble gases are deliberately excluded from this visualization heuristic.
                if [2, 10, 18, 36, 54, 86, 118].contains(&a.atomic_number)
                    || [2, 10, 18, 36, 54, 86, 118].contains(&b.atomic_number)
                {
                    continue;
                }
                let cutoff = 1.2
                    * (COVALENT_RADII[a.atomic_number as usize]
                        + COVALENT_RADII[b.atomic_number as usize]);
                let distance = a.position.distance(b.position);
                if distance > 0.1 && distance <= cutoff {
                    bonds.push(DisplayBond {
                        atoms: [j, i],
                        order: 1,
                    });
                }
            }
        }
        Self {
            atoms,
            bonds,
            center,
            radius,
        }
    }
    pub fn set_bonds(&mut self, bonds: Vec<DisplayBond>) -> Result<(), String> {
        let mut pairs = std::collections::BTreeSet::new();
        for bond in &bonds {
            let [a, b] = bond.atoms;
            if a >= self.atoms.len()
                || b >= self.atoms.len()
                || a == b
                || !(1..=3).contains(&bond.order)
                || !pairs.insert((a.min(b), a.max(b)))
            {
                return Err("valence_protocol".into());
            }
        }
        self.bonds = bonds;
        Ok(())
    }
    pub fn bond_instance_count(&self) -> u32 {
        self.bonds
            .iter()
            .map(|bond| u32::from(bond.order) * 2)
            .sum()
    }
    pub fn instances(&self, selected: Option<usize>, show_bonds: bool) -> Vec<Instance> {
        self.instances_for_camera(selected, show_bonds, &Camera::default(), 1.0)
    }
    pub fn instances_for_camera(
        &self,
        selected: Option<usize>,
        show_bonds: bool,
        camera: &Camera,
        aspect: f32,
    ) -> Vec<Instance> {
        let mut instances = Vec::new();
        for (index, atom) in self.atoms.iter().enumerate() {
            let color = if selected == Some(index) {
                [1.0, 0.58, 0.08, 1.0]
            } else {
                atom.color
            };
            instances.push(Instance::new(
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(atom.radius),
                    Quat::IDENTITY,
                    atom.position - self.center,
                ),
                color,
            ));
        }
        if show_bonds {
            let eye = camera.eye(self.radius, aspect);
            for bond in &self.bonds {
                let [i, j] = bond.atoms;
                let a = &self.atoms[i];
                let b = &self.atoms[j];
                let delta = b.position - a.position;
                let length = delta.length();
                let direction = delta / length;
                let rotation = Quat::from_rotation_arc(Vec3::Y, direction);
                // Offset perpendicular to the view and bond, so parallel rods remain
                // separated when orbiting. The fallback handles an end-on bond.
                let view = (eye - ((a.position + b.position) * 0.5 - self.center)).normalize();
                let mut side = direction.cross(view);
                if side.length_squared() < 1e-8 {
                    side = direction.cross(if direction.y.abs() < 0.9 {
                        Vec3::Y
                    } else {
                        Vec3::X
                    });
                }
                side = side.normalize();
                let (radius, spacing) = match bond.order {
                    1 => (0.085, 0.0),
                    2 => (0.052, 0.23),
                    _ => (0.044, 0.16),
                };
                for rod in 0..bond.order {
                    let offset =
                        side * (f32::from(rod) - f32::from(bond.order - 1) * 0.5) * spacing;
                    // Each rod retains the two endpoint element colors.
                    for (fraction, color) in [(0.25, a.color), (0.75, b.color)] {
                        instances.push(Instance::new(
                            Mat4::from_scale_rotation_translation(
                                Vec3::new(radius, length * 0.5, radius),
                                rotation,
                                a.position + delta * fraction + offset - self.center,
                            ),
                            color,
                        ));
                    }
                }
            }
        }

        instances
    }
    pub fn pick(&self, camera: &Camera, aspect: f32, x: f32, y: f32) -> Option<usize> {
        let inverse = camera.matrix(self.radius, aspect).inverse();
        let start = inverse.project_point3(Vec3::new(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0));
        let end = inverse.project_point3(Vec3::new(x * 2.0 - 1.0, 1.0 - y * 2.0, 1.0));
        let direction = (end - start).normalize();
        self.atoms
            .iter()
            .enumerate()
            .filter_map(|(index, atom)| {
                ray_sphere(start, direction, atom.position - self.center, atom.radius)
                    .map(|distance| (index, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }
    pub fn symbol(&self, index: usize) -> &'static str {
        SYMBOLS[self.atoms[index].atomic_number as usize]
    }
}
fn color(z: u8) -> [f32; 4] {
    match z {
        1 => [0.88, 0.91, 0.96, 1.0],
        6 => [0.18, 0.23, 0.30, 1.0],
        7 => [0.12, 0.30, 0.87, 1.0],
        8 => [0.86, 0.08, 0.09, 1.0],
        9 | 17 => [0.13, 0.65, 0.24, 1.0],
        15 => [0.95, 0.39, 0.08, 1.0],
        16 => [0.91, 0.74, 0.08, 1.0],
        35 => [0.52, 0.15, 0.09, 1.0],
        53 => [0.46, 0.21, 0.65, 1.0],
        _ => [0.48, 0.58, 0.75, 1.0],
    }
}
pub fn ray_sphere(origin: Vec3, direction: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let offset = origin - center;
    let b = offset.dot(direction);
    let discriminant = b * b - (offset.length_squared() - radius * radius);
    if discriminant < 0.0 {
        return None;
    }
    let near = -b - discriminant.sqrt();
    let far = -b + discriminant.sqrt();
    if near >= 0.0 {
        Some(near)
    } else if far >= 0.0 {
        Some(far)
    } else {
        None
    }
}
#[derive(Clone, Copy)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.85,
            pitch: 0.28,
            zoom: 1.0,
        }
    }
}
impl Camera {
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw + dx * 0.008).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.48, 1.48);
    }
    pub fn zoom_by(&mut self, delta: f32) {
        self.zoom = (self.zoom * (delta.clamp(-400.0, 400.0) * 0.002).exp()).clamp(0.35, 8.0);
    }
    pub fn eye(&self, radius: f32, aspect: f32) -> Vec3 {
        let half_angle = (std::f32::consts::FRAC_PI_8.tan() * aspect.min(1.0)).atan();
        let distance = radius.max(0.5) / half_angle.sin() * 1.2 * self.zoom;
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        ) * distance
    }
    pub fn matrix(&self, radius: f32, aspect: f32) -> Mat4 {
        let eye = self.eye(radius, aspect);
        let projection = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            aspect,
            radius.max(0.5) * 0.005,
            eye.length() + radius.max(0.5) * 30.0,
        );
        projection * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn water_and_hydrogen_have_only_geometry_hint_bonds() {
        let water =
            Document::from_xyz("3\nWater\nO 0 0 0\nH .7586 0 .5043\nH -.7586 0 .5043", 0, 0)
                .unwrap();
        let scene = Scene::from_document(Some(&water));
        assert_eq!(scene.bonds.len(), 2);
        assert_eq!(scene.instances(None, true).len(), 7);
        assert_eq!(scene.instances(None, false).len(), 3);
        let h2 = Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 .74", 0, 0).unwrap();
        assert_eq!(Scene::from_document(Some(&h2)).bonds.len(), 1);
        let separated = Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 3", 0, 0).unwrap();
        assert!(Scene::from_document(Some(&separated)).bonds.is_empty());
    }
    #[test]
    fn multiple_bond_rods_are_finite_distinct_and_counted_for_every_view() {
        let doc = Document::from_xyz("2\nC2\nC -.6 0 0\nC .6 0 0", 0, 0).unwrap();
        let mut scene = Scene::from_document(Some(&doc));
        for order in 1..=3 {
            scene
                .set_bonds(vec![DisplayBond {
                    atoms: [0, 1],
                    order,
                }])
                .unwrap();
            assert_eq!(scene.bond_instance_count(), u32::from(order) * 2);
            assert_eq!(scene.instances(None, false).len(), 2);
            for aspect in [0.35, 1.0, 3.0] {
                for (yaw, pitch) in [
                    (0.0, 0.0),
                    (0.85, 0.28),
                    (std::f32::consts::FRAC_PI_2, 0.0),
                    (3.0, 1.48),
                ] {
                    let camera = Camera {
                        yaw,
                        pitch,
                        zoom: 1.0,
                    };
                    let instances = scene.instances_for_camera(None, true, &camera, aspect);
                    assert_eq!(instances.len(), 2 + usize::from(order) * 2);
                    assert!(instances
                        .iter()
                        .all(|i| Mat4::from_cols_array_2d(&i.model).is_finite()
                            && Mat4::from_cols_array_2d(&i.normal).is_finite()));
                    let matrix = camera.matrix(scene.radius, aspect);
                    let points: Vec<_> = (0..order)
                        .map(|rod| {
                            let model = Mat4::from_cols_array_2d(
                                &instances[2 + usize::from(rod) * 2].model,
                            );
                            matrix.project_point3(model.w_axis.truncate())
                        })
                        .collect();
                    for pair in points.windows(2) {
                        assert!(pair[0].truncate().distance(pair[1].truncate()) > 0.005);
                    }
                }
            }
        }
        let saved = scene.bonds.clone();
        assert!(scene
            .set_bonds(vec![DisplayBond {
                atoms: [0, 2],
                order: 2
            }])
            .is_err());
        assert!(scene
            .set_bonds(vec![DisplayBond {
                atoms: [0, 1],
                order: 4
            }])
            .is_err());
        assert!(scene.set_bonds(vec![saved[0], saved[0]]).is_err());
        assert_eq!(scene.bonds, saved);
    }
    #[test]
    fn fit_keeps_atoms_inside_portrait_and_landscape_frusta() {
        let document = Document::from_xyz("2\nH2\nH -12 0 0\nH 12 0 0", 0, 0).unwrap();
        let scene = Scene::from_document(Some(&document));
        for aspect in [0.35, 1.0, 3.0] {
            let camera = Camera::default();
            let matrix = camera.matrix(scene.radius, aspect);
            for atom in &scene.atoms {
                let clip = matrix.project_point3(atom.position - scene.center);
                assert!(clip.x.abs() < 1.0 && clip.y.abs() < 1.0 && clip.z > 0.0 && clip.z < 1.0);
            }
        }
    }
    #[test]
    fn picking_returns_front_atom_and_tracks_camera_rotation() {
        let doc = Document::from_xyz("2\nH2\nH 0 0 -0.4\nH 0 0 0.4", 0, 0).unwrap();
        let scene = Scene::from_document(Some(&doc));
        let mut camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
        };
        assert_eq!(scene.pick(&camera, 1.0, 0.5, 0.5), Some(1));
        camera.yaw = std::f32::consts::PI;
        assert_eq!(scene.pick(&camera, 1.0, 0.5, 0.5), Some(0));
        assert_eq!(scene.pick(&camera, 1.0, 0.01, 0.01), None);
    }
    #[test]
    fn ray_picking_and_camera_limits_are_finite() {
        assert_eq!(
            ray_sphere(Vec3::new(0.0, 0.0, 3.0), -Vec3::Z, Vec3::ZERO, 1.0),
            Some(2.0)
        );
        assert_eq!(
            ray_sphere(Vec3::new(0.0, 0.0, 3.0), Vec3::Z, Vec3::ZERO, 1.0),
            None
        );
        let mut camera = Camera::default();
        camera.orbit(1e6, 1e6);
        camera.zoom_by(1e6);
        assert!(camera.matrix(1.0, 1.0).is_finite());
        assert!(camera.pitch <= 1.48 && camera.zoom <= 8.0);
    }
}
