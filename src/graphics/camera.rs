use glam::{Mat4, Vec3};

/// Degrees of view rotation per pixel of mouse movement while looking around.
pub const LOOK_SENSITIVITY: f32 = 0.15;

/// Shared trait for 3D viewport cameras.
pub trait Camera {
    fn view_matrix(&self) -> Mat4;
    fn orbit(&mut self, dx: f32, dy: f32);
    fn pan(&mut self, dx: f32, dy: f32);
    fn move_camera(&mut self, forward: f32, right: f32, up: f32, dt: f32);
    fn zoom(&mut self, delta: f32);
    fn reset(&mut self);
    fn draws_marker(&self) -> bool {
        false
    }
    fn marker_target(&self) -> Vec3 {
        Vec3::ZERO
    }
}

/// Orbit camera that rotates and zooms around a look-at target.
#[derive(Debug, Clone, PartialEq)]
pub struct OrbitCamera {
    azimuth: f32,
    elevation: f32,
    distance: f32,
    target: [f32; 3],
}

impl OrbitCamera {
    pub fn new(distance: f32) -> Self {
        Self {
            azimuth: 30.0,
            elevation: 20.0,
            distance,
            target: [0.0, 0.0, 0.0],
        }
    }

    pub fn azimuth(&self) -> f32 {
        self.azimuth
    }

    pub fn elevation(&self) -> f32 {
        self.elevation
    }

    pub fn distance(&self) -> f32 {
        self.distance
    }

    pub fn target(&self) -> [f32; 3] {
        self.target
    }

    pub fn get_state(&self) -> (f32, f32, f32, [f32; 3]) {
        (self.azimuth, self.elevation, self.distance, self.target)
    }

    pub fn set_state(&mut self, azimuth: f32, elevation: f32, distance: f32, target: [f32; 3]) {
        self.azimuth = azimuth;
        self.elevation = elevation;
        self.distance = distance;
        self.target = target;
    }

    pub fn set_from_free(&mut self, free: &FreeCamera) {
        self.azimuth = free.yaw();
        self.elevation = free.pitch().clamp(-89.0, 89.0);
        let dir = free.forward();
        let eye = free.position();
        self.target = [
            eye[0] + dir.x * self.distance,
            eye[1] + dir.y * self.distance,
            eye[2] + dir.z * self.distance,
        ];
    }
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self::new(6.0)
    }
}

impl Camera for OrbitCamera {
    fn view_matrix(&self) -> Mat4 {
        let az = self.azimuth.to_radians();
        let el = self.elevation.to_radians();
        let target = Vec3::from_array(self.target);
        let eye = Vec3::new(
            target.x + self.distance * el.cos() * az.sin(),
            target.y + self.distance * el.sin(),
            target.z + self.distance * el.cos() * az.cos(),
        );
        glam::camera::rh::view::look_at_mat4(eye, target, Vec3::new(0.0, 1.0, 0.0))
    }

    fn orbit(&mut self, dx: f32, dy: f32) {
        self.azimuth -= dx * LOOK_SENSITIVITY;
        self.elevation += dy * LOOK_SENSITIVITY;
        self.elevation = self.elevation.clamp(-89.0, 89.0);
    }

    fn pan(&mut self, dx: f32, dy: f32) {
        let az = self.azimuth.to_radians();
        let rx = az.cos();
        let rz = -az.sin();
        let scale = self.distance * 0.002;
        self.target[0] -= (rx * dx) * scale;
        self.target[1] += dy * scale;
        self.target[2] -= (rz * dx) * scale;
    }

    fn move_camera(&mut self, forward: f32, right: f32, up: f32, dt: f32) {
        let az = self.azimuth.to_radians();
        let el = self.elevation.to_radians();
        let fx = -el.cos() * az.sin();
        let fy = -el.sin();
        let fz = -el.cos() * az.cos();
        let rx = az.cos();
        let rz = -az.sin();
        let ux = -rz * fy;
        let uy = rz * fx - rx * fz;
        let uz = rx * fy;
        let step = self.distance * 0.9 * dt;
        self.target[0] += (fx * forward + rx * right + ux * up) * step;
        self.target[1] += (fy * forward + uy * up) * step;
        self.target[2] += (fz * forward + rz * right + uz * up) * step;
    }

    fn zoom(&mut self, delta: f32) {
        self.distance = (self.distance - delta * 0.5).max(0.5);
    }

    fn reset(&mut self) {
        let dist = self.distance;
        *self = OrbitCamera::new(dist);
    }

    fn draws_marker(&self) -> bool {
        true
    }

    fn marker_target(&self) -> Vec3 {
        Vec3::from_array(self.target)
    }
}

/// First-person fly / free camera with WASD movement.
#[derive(Debug, Clone, PartialEq)]
pub struct FreeCamera {
    position: [f32; 3],
    yaw: f32,
    pitch: f32,
}

impl FreeCamera {
    pub fn new() -> Self {
        Self {
            position: [0.0, 2.0, 6.0],
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    pub fn position(&self) -> [f32; 3] {
        self.position
    }

    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    pub fn forward(&self) -> Vec3 {
        let yaw = self.yaw.to_radians();
        let pitch = self.pitch.to_radians();
        Vec3::new(-pitch.cos() * yaw.sin(), -pitch.sin(), -pitch.cos() * yaw.cos())
    }

    pub fn set_from_orbit(&mut self, orbit: &OrbitCamera) {
        let az = orbit.azimuth().to_radians();
        let el = orbit.elevation().to_radians();
        let target = orbit.target();
        let distance = orbit.distance();
        self.position = [
            target[0] + distance * el.cos() * az.sin(),
            target[1] + distance * el.sin(),
            target[2] + distance * el.cos() * az.cos(),
        ];
        self.yaw = orbit.azimuth();
        self.pitch = orbit.elevation();
    }
}

impl Default for FreeCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl Camera for FreeCamera {
    fn view_matrix(&self) -> Mat4 {
        let dir = self.forward();
        let eye = Vec3::from_array(self.position);
        glam::camera::rh::view::look_at_mat4(eye, eye + dir, Vec3::new(0.0, 1.0, 0.0))
    }

    fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * LOOK_SENSITIVITY;
        self.pitch += dy * LOOK_SENSITIVITY;
        self.pitch = self.pitch.clamp(-89.0, 89.0);
    }

    fn pan(&mut self, dx: f32, dy: f32) {
        let yaw = self.yaw.to_radians();
        let rx = yaw.cos();
        let rz = -yaw.sin();
        let scale = 0.01;
        self.position[0] -= rx * dx * scale;
        self.position[1] += dy * scale;
        self.position[2] -= rz * dx * scale;
    }

    fn move_camera(&mut self, forward: f32, right: f32, up: f32, dt: f32) {
        let dir = self.forward();
        let yaw = self.yaw.to_radians();
        let rx = yaw.cos();
        let rz = -yaw.sin();
        let ux = -rz * dir.y;
        let uy = rz * dir.x - rx * dir.z;
        let uz = rx * dir.y;
        let step = 4.0 * dt;
        self.position[0] += (dir.x * forward + rx * right + ux * up) * step;
        self.position[1] += (dir.y * forward + uy * up) * step;
        self.position[2] += (dir.z * forward + rz * right + uz * up) * step;
    }

    fn zoom(&mut self, delta: f32) {
        let dir = self.forward();
        let step = delta * 0.5;
        self.position[0] += dir.x * step;
        self.position[1] += dir.y * step;
        self.position[2] += dir.z * step;
    }

    fn reset(&mut self) {
        *self = FreeCamera::new();
    }
}

/// Perspective projection matrix used for rendering the 3D scene.
pub fn perspective_projection(aspect: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective(45.0f32.to_radians(), aspect, 0.1, 500.0)
}

/// Computes a world-space ray (origin, normalized direction) from viewport normalized coordinates `(x, y)` in `[0, 1]`.
pub fn ray_from_screen(view: &Mat4, proj: &Mat4, screen_norm: [f32; 2]) -> (Vec3, Vec3) {
    let ndc_x = screen_norm[0] * 2.0 - 1.0;
    let ndc_y = 1.0 - screen_norm[1] * 2.0; // Y is inverted in NDC

    let inv_vp = (*proj * *view).inverse();
    let near_pt = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
    let far_pt = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
    let dir = (far_pt - near_pt).normalize();
    (near_pt, dir)
}
