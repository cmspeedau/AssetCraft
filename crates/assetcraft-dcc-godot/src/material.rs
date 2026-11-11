//! Godot's view of a PBR material.

use std::path::{Path, PathBuf};

use assetcraft_dcc_blender::material::{Binding, MaterialSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotMaterial {
    pub name: String,
    pub albedo: Option<PathBuf>,
    pub metallic: Option<PathBuf>,
    pub roughness: Option<PathBuf>,
    pub normal: Option<PathBuf>,
    pub emission: Option<PathBuf>,
    pub ambient_occlusion: Option<PathBuf>,
    pub orm: Option<PathBuf>,
}

impl GodotMaterial {
    pub fn from_material_spec(spec: &MaterialSpec) -> Self {
        let mut material = Self {
            name: spec.name.clone(),
            albedo: None,
            metallic: None,
            roughness: None,
            normal: None,
            emission: None,
            ambient_occlusion: None,
            orm: None,
        };
        for map in &spec.maps {
            match map.binding {
                Binding::BaseColor => material.albedo = Some(map.target.clone()),
                Binding::Metallic => material.metallic = Some(map.target.clone()),
                Binding::Roughness | Binding::InvertedRoughness => {
                    if material.roughness.is_none() {
                        material.roughness = Some(map.target.clone());
                    }
                }
                Binding::Normal => material.normal = Some(map.target.clone()),
                Binding::Emission => material.emission = Some(map.target.clone()),
                Binding::AmbientOcclusion => material.ambient_occlusion = Some(map.target.clone()),
                _ => {}
            }
        }
        material
    }

    pub fn can_be_orm(&self) -> bool {
        self.ambient_occlusion.is_some() || self.roughness.is_some() || self.metallic.is_some()
    }

    pub fn orm_texture(&self) -> Option<&PathBuf> {
        self.orm.as_ref()
    }
}

pub fn orm_pack(
    export_dir: &Path,
    material_name: &str,
    ambient_occlusion: Option<&Path>,
    roughness: Option<&Path>,
    metallic: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    if ambient_occlusion.is_none() && roughness.is_none() && metallic.is_none() {
        return Ok(None);
    }
    use image::{GenericImageView, ImageBuffer, Rgb, RgbImage};
    let ao_img = ambient_occlusion.and_then(|p| image::open(export_dir.join(p)).ok());
    let rough_img = roughness.and_then(|p| image::open(export_dir.join(p)).ok());
    let metal_img = metallic.and_then(|p| image::open(export_dir.join(p)).ok());

    let (width, height) = [ao_img.as_ref(), rough_img.as_ref(), metal_img.as_ref()]
        .iter()
        .filter_map(|img| img.map(|i| i.dimensions()))
        .max_by_key(|d| d.0 * d.1)
        .unwrap_or((1, 1));

    let mut out: RgbImage = ImageBuffer::new(width, height);
    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let ao = ao_img
            .as_ref()
            .map(|img| img.get_pixel(x.min(img.width() - 1), y.min(img.height() - 1))[0])
            .unwrap_or(255);
        let rough = rough_img
            .as_ref()
            .map(|img| img.get_pixel(x.min(img.width() - 1), y.min(img.height() - 1))[0])
            .unwrap_or(128);
        let metal = metal_img
            .as_ref()
            .map(|img| img.get_pixel(x.min(img.width() - 1), y.min(img.height() - 1))[0])
            .unwrap_or(0);
        *pixel = Rgb([ao, rough, metal]);
    }

    let relative = PathBuf::from("textures").join(format!("{material_name}_orm.png"));
    let absolute = export_dir.join(&relative);
    if let Some(parent) = absolute.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    out.save(&absolute).map_err(|e| e.to_string())?;
    Ok(Some(relative))
}

#[cfg(test)]
mod tests {
    use super::*;
    use assetcraft_core::TextureRole;
    use assetcraft_dcc_blender::material::{Binding, MaterialSpec, TextureMap};
    use std::path::PathBuf;

    #[test]
    fn from_material_spec_maps_roles_to_godot_slots() {
        let spec = MaterialSpec {
            name: "rock".to_owned(),
            maps: vec![
                TextureMap {
                    role: TextureRole::BaseColor,
                    binding: Binding::BaseColor,
                    target: PathBuf::from("textures/rock_basecolor.png"),
                },
                TextureMap {
                    role: TextureRole::Normal,
                    binding: Binding::Normal,
                    target: PathBuf::from("textures/rock_normal.png"),
                },
                TextureMap {
                    role: TextureRole::Roughness,
                    binding: Binding::Roughness,
                    target: PathBuf::from("textures/rock_roughness.png"),
                },
                TextureMap {
                    role: TextureRole::Metallic,
                    binding: Binding::Metallic,
                    target: PathBuf::from("textures/rock_metallic.png"),
                },
                TextureMap {
                    role: TextureRole::AmbientOcclusion,
                    binding: Binding::AmbientOcclusion,
                    target: PathBuf::from("textures/rock_ao.png"),
                },
            ],
        };
        let m = GodotMaterial::from_material_spec(&spec);
        assert_eq!(
            m.albedo.as_deref(),
            Some(Path::new("textures/rock_basecolor.png"))
        );
        assert_eq!(
            m.normal.as_deref(),
            Some(Path::new("textures/rock_normal.png"))
        );
        assert_eq!(
            m.roughness.as_deref(),
            Some(Path::new("textures/rock_roughness.png"))
        );
        assert_eq!(
            m.metallic.as_deref(),
            Some(Path::new("textures/rock_metallic.png"))
        );
        assert_eq!(
            m.ambient_occlusion.as_deref(),
            Some(Path::new("textures/rock_ao.png"))
        );
        assert!(m.can_be_orm());
    }

    #[test]
    fn orm_pack_returns_none_when_no_maps() {
        let dir = tempfile::tempdir().unwrap();
        let result = orm_pack(dir.path(), "rock", None, None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn orm_pack_writes_a_png_with_correct_channels() {
        use image::{ImageBuffer, Rgb};
        let dir = tempfile::tempdir().unwrap();
        let tex = dir.path().join("textures");
        std::fs::create_dir_all(&tex).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([10u8, 20, 30]))
            .save(tex.join("ao.png"))
            .unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([40u8, 50, 60]))
            .save(tex.join("rough.png"))
            .unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([70u8, 80, 90]))
            .save(tex.join("metal.png"))
            .unwrap();

        let rel = orm_pack(
            dir.path(),
            "rock",
            Some(Path::new("textures/ao.png")),
            Some(Path::new("textures/rough.png")),
            Some(Path::new("textures/metal.png")),
        )
        .unwrap()
        .unwrap();
        assert_eq!(rel, PathBuf::from("textures/rock_orm.png"));
        let img = image::open(dir.path().join(&rel)).unwrap().to_rgb8();
        assert_eq!(img.get_pixel(0, 0), &Rgb([10, 40, 70]));
    }

    #[test]
    fn orm_pack_fills_missing_channels_with_defaults() {
        use image::{ImageBuffer, Rgb};
        let dir = tempfile::tempdir().unwrap();
        let tex = dir.path().join("textures");
        std::fs::create_dir_all(&tex).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([5u8, 5, 5]))
            .save(tex.join("rough.png"))
            .unwrap();

        let rel = orm_pack(
            dir.path(),
            "rock",
            None,
            Some(Path::new("textures/rough.png")),
            None,
        )
        .unwrap()
        .unwrap();
        let img = image::open(dir.path().join(&rel)).unwrap().to_rgb8();
        assert_eq!(img.get_pixel(0, 0), &Rgb([255, 5, 0]));
    }
}
