//! HCDF to DeviceData conversion utilities
//!
//! Shared conversion functions for transforming dendrite-core HCDF types
//! into dendrite-scene visualization types.

use dendrite_core::hcdf::{
    Comp, Fov as HcdfFov, Frame, Mcu, Port as HcdfPort, Sensor as HcdfSensor, Visual,
    parse_hex_color,
};

use crate::types::{
    AxisAlignData, DeviceData, DeviceStatus, FovData, FrameData, GeometryData, PortData,
    SensorData, VisualData,
};

/// Convert MCU to DeviceData
pub fn mcu_to_device_data(mcu: &Mcu) -> DeviceData {
    let pose = mcu.pose_cg.as_ref().and_then(|s| parse_pose_string(s));

    DeviceData {
        id: mcu.hwid.clone().unwrap_or_else(|| mcu.name.clone()),
        name: mcu.name.clone(),
        board: mcu.board.clone(),
        ip: mcu
            .discovered
            .as_ref()
            .map(|d| d.ip.clone())
            .unwrap_or_default(),
        port: mcu.discovered.as_ref().and_then(|d| d.port),
        status: if mcu.discovered.is_some() {
            DeviceStatus::Online
        } else {
            DeviceStatus::Unknown
        },
        version: mcu.software.as_ref().and_then(|s| s.version.clone()),
        position: pose.map(|p| [p[0], p[1], p[2]]),
        orientation: pose.map(|p| [p[3], p[4], p[5]]),
        model_path: mcu.model.as_ref().map(|m| m.href.clone()),
        visuals: mcu.visual.iter().map(visual_to_visual_data).collect(),
        frames: mcu.frame.iter().map(frame_to_frame_data).collect(),
        ports: Vec::new(), // MCUs don't have ports
        sensors: Vec::new(), // MCUs don't have sensors
        last_seen: mcu.discovered.as_ref().and_then(|d| d.last_seen.clone()),
    }
}

/// Convert Comp to DeviceData
pub fn comp_to_device_data(comp: &Comp) -> DeviceData {
    let pose = comp.pose_cg.as_ref().and_then(|s| parse_pose_string(s));

    DeviceData {
        id: comp.hwid.clone().unwrap_or_else(|| comp.name.clone()),
        name: comp.name.clone(),
        board: comp.board.clone(),
        ip: comp
            .discovered
            .as_ref()
            .map(|d| d.ip.clone())
            .unwrap_or_default(),
        port: comp.discovered.as_ref().and_then(|d| d.port),
        status: if comp.discovered.is_some() {
            DeviceStatus::Online
        } else {
            DeviceStatus::Unknown
        },
        version: comp.software.as_ref().and_then(|s| s.version.clone()),
        position: pose.map(|p| [p[0], p[1], p[2]]),
        orientation: pose.map(|p| [p[3], p[4], p[5]]),
        model_path: comp.model.as_ref().map(|m| m.href.clone()),
        visuals: comp.visual.iter().map(visual_to_visual_data).collect(),
        frames: comp.frame.iter().map(frame_to_frame_data).collect(),
        ports: comp.port.iter().map(port_to_port_data).collect(),
        sensors: comp
            .sensor
            .iter()
            .flat_map(sensors_to_sensor_data_vec)
            .collect(),
        last_seen: comp.discovered.as_ref().and_then(|d| d.last_seen.clone()),
    }
}

/// Convert Visual to VisualData
pub fn visual_to_visual_data(v: &Visual) -> VisualData {
    VisualData {
        name: v.name.clone(),
        toggle: v.toggle.clone(),
        pose: v.parse_pose().map(|p| p.to_array()),
        model_path: v.model.as_ref().map(|m| m.href.clone()),
        model_sha: v.model.as_ref().and_then(|m| m.sha.clone()),
    }
}

/// Convert Frame to FrameData
pub fn frame_to_frame_data(f: &Frame) -> FrameData {
    FrameData {
        name: f.name.clone(),
        description: f.description.clone(),
        pose: f.parse_pose().map(|p| p.to_array()),
    }
}

/// Convert Port to PortData
pub fn port_to_port_data(p: &HcdfPort) -> PortData {
    PortData {
        name: p.name.clone(),
        port_type: p.port_type.clone(),
        pose: p.parse_pose().map(|pp| pp.to_array()),
        geometry: p
            .geometry
            .iter()
            .filter_map(geometry_to_geometry_data)
            .collect(),
        visual_name: p.visual.clone(),
        mesh_name: p.mesh.clone(),
    }
}

/// Flatten a Sensor struct into multiple SensorData
pub fn sensors_to_sensor_data_vec(s: &HcdfSensor) -> Vec<SensorData> {
    let mut result = Vec::new();

    // Process inertial sensors
    for inertial in &s.inertial {
        let axis = inertial
            .driver
            .as_ref()
            .and_then(|d| d.axis_align.as_ref().and_then(|a| a.parse_axes()));
        let axis_data = axis.map(|(x, y, z)| AxisAlignData {
            x: format!("{:?}", x),
            y: format!("{:?}", y),
            z: format!("{:?}", z),
        });
        result.push(SensorData {
            name: s.name.clone(),
            category: "inertial".to_string(),
            sensor_type: inertial.sensor_type.clone(),
            driver: inertial.driver.as_ref().map(|d| d.name.clone()),
            pose: inertial.parse_pose().map(|p| p.to_array()),
            axis_align: axis_data,
            geometry: None,
            fovs: Vec::new(),
        });
    }

    // Process EM sensors
    for em in &s.em {
        let axis = em
            .driver
            .as_ref()
            .and_then(|d| d.axis_align.as_ref().and_then(|a| a.parse_axes()));
        let axis_data = axis.map(|(x, y, z)| AxisAlignData {
            x: format!("{:?}", x),
            y: format!("{:?}", y),
            z: format!("{:?}", z),
        });
        result.push(SensorData {
            name: s.name.clone(),
            category: "em".to_string(),
            sensor_type: em.sensor_type.clone(),
            driver: em.driver.as_ref().map(|d| d.name.clone()),
            pose: em.parse_pose().map(|p| p.to_array()),
            axis_align: axis_data,
            geometry: None,
            fovs: Vec::new(),
        });
    }

    // Process optical sensors
    for optical in &s.optical {
        let fovs: Vec<FovData> = optical.fov.iter().map(fov_to_fov_data).collect();
        result.push(SensorData {
            name: s.name.clone(),
            category: "optical".to_string(),
            sensor_type: optical.sensor_type.clone(),
            driver: optical.driver.as_ref().map(|d| d.name.clone()),
            pose: optical.parse_pose().map(|p| p.to_array()),
            axis_align: None,
            geometry: None,
            fovs,
        });
    }

    // Process RF sensors
    for rf in &s.rf {
        result.push(SensorData {
            name: s.name.clone(),
            category: "rf".to_string(),
            sensor_type: rf.sensor_type.clone(),
            driver: rf.driver.as_ref().map(|d| d.name.clone()),
            pose: rf.parse_pose().map(|p| p.to_array()),
            axis_align: None,
            geometry: None,
            fovs: Vec::new(),
        });
    }

    // Process force sensors
    for force in &s.force {
        result.push(SensorData {
            name: s.name.clone(),
            category: "force".to_string(),
            sensor_type: force.sensor_type.clone(),
            driver: force.driver.as_ref().map(|d| d.name.clone()),
            pose: force.parse_pose().map(|p| p.to_array()),
            axis_align: None,
            geometry: None,
            fovs: Vec::new(),
        });
    }

    // Process chemical sensors
    for chem in &s.chemical {
        result.push(SensorData {
            name: s.name.clone(),
            category: "chemical".to_string(),
            sensor_type: chem.sensor_type.clone(),
            driver: chem.driver.as_ref().map(|d| d.name.clone()),
            pose: chem.parse_pose().map(|p| p.to_array()),
            axis_align: None,
            geometry: None,
            fovs: Vec::new(),
        });
    }

    result
}

/// Convert FOV to FovData
pub fn fov_to_fov_data(fov: &HcdfFov) -> FovData {
    let pose = fov.parse_pose();
    let color = fov.color.as_ref().and_then(|c| parse_hex_color(c));

    let geometry = fov
        .geometry
        .as_ref()
        .and_then(|g| geometry_to_geometry_data(g));

    FovData {
        name: fov.name.clone(),
        color: color.map(|(r, g, b)| [r, g, b]),
        pose: pose.map(|p| p.to_array()),
        geometry,
    }
}

/// Convert HCDF Geometry to GeometryData
pub fn geometry_to_geometry_data(g: &dendrite_core::hcdf::Geometry) -> Option<GeometryData> {
    if let Some(b) = &g.box_geom {
        let size = b.parse_size()?;
        return Some(GeometryData::Box { size });
    }
    if let Some(c) = &g.cylinder {
        return Some(GeometryData::Cylinder {
            radius: c.radius,
            length: c.length,
        });
    }
    if let Some(s) = &g.sphere {
        return Some(GeometryData::Sphere { radius: s.radius });
    }
    if let Some(pf) = &g.pyramidal_frustum {
        return Some(GeometryData::PyramidalFrustum {
            near: pf.near,
            far: pf.far,
            hfov: pf.hfov,
            vfov: pf.vfov,
        });
    }
    if let Some(cf) = &g.conical_frustum {
        return Some(GeometryData::ConicalFrustum {
            near: cf.near,
            far: cf.far,
            fov: cf.fov,
        });
    }
    None
}

/// Parse pose string "x y z roll pitch yaw" to [f64; 6]
pub fn parse_pose_string(s: &str) -> Option<[f64; 6]> {
    let parts: Vec<f64> = s
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() >= 6 {
        Some([parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]])
    } else {
        None
    }
}
