//! The PBR material graph a set of texture maps describes.
//!
//! A [`MaterialSpec`] lists the maps that make up one material, each with the
//! [`Binding`] that says how it wires into the Principled BSDF. The binding is
//! deliberately semantic rather than a Blender socket name: which socket a
//! binding lands on, and which node sits in between, is the script generator's
//! business and changes between Blender versions. Keeping the intent here means
//! the graph can be tested without Blender.

use crate::TextureRole;

/// How a texture map wires into the material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// The Principled BSDF's base colour.
    BaseColor,
    /// The Principled BSDF's roughness.
    Roughness,
    /// The Principled BSDF's metallic.
    Metallic,
    /// The Principled BSDF's specular.
    Specular,
    /// The Principled BSDF's alpha, with the material set to blend.
    Alpha,
    /// The Principled BSDF's emission colour, with strength raised.
    Emission,
    /// Through a Normal Map node into the Principled BSDF's normal.
    Normal,
    /// Inverted (`1 - x`) into the Principled BSDF's roughness, for a
    /// glossiness map.
    InvertedRoughness,
    /// Multiplied into the base colour, for an ambient occlusion map.
    AmbientOcclusion,
    /// Through a Displacement node into the material output's displacement.
    Displacement,
}

impl Binding {
    /// The stable name written into the generated script.
    pub const fn as_str(self) -> &'static str {
        match self {
            Binding::BaseColor => "BaseColor",
            Binding::Roughness => "Roughness",
            Binding::Metallic => "Metallic",
            Binding::Specular => "Specular",
            Binding::Alpha => "Alpha",
            Binding::Emission => "Emission",
            Binding::Normal => "Normal",
            Binding::InvertedRoughness => "InvertedRoughness",
            Binding::AmbientOcclusion => "AmbientOcclusion",
            Binding::Displacement => "Displacement",
        }
    }

    /// Where the map falls in the generated script, base colour first.
    ///
    /// Base colour is placed before the ambient occlusion that multiplies into
    /// it, so the script sees the base colour node when it reaches the AO map.
    pub const fn order(self) -> u8 {
        match self {
            Binding::BaseColor => 0,
            Binding::AmbientOcclusion => 1,
            Binding::Metallic => 2,
            Binding::Roughness => 3,
            Binding::InvertedRoughness => 4,
            Binding::Specular => 5,
            Binding::Normal => 6,
            Binding::Displacement => 7,
            Binding::Alpha => 8,
            Binding::Emission => 9,
        }
    }

    /// The material channel the map drives.
    ///
    /// Two maps with the same channel would fight over one socket, so a
    /// material keeps only the first. Roughness and glossiness share a channel
    /// because glossiness is roughness inverted; ambient occlusion does not
    /// share base colour's, because it multiplies *into* it rather than
    /// replacing it.
    pub const fn channel(self) -> &'static str {
        match self {
            Binding::Roughness | Binding::InvertedRoughness => "roughness",
            Binding::BaseColor => "baseColor",
            Binding::AmbientOcclusion => "ambientOcclusion",
            Binding::Metallic => "metallic",
            Binding::Specular => "specular",
            Binding::Alpha => "alpha",
            Binding::Emission => "emission",
            Binding::Normal => "normal",
            Binding::Displacement => "displacement",
        }
    }
}

impl TextureRole {
    /// How a texture in this role wires into the material, if it does at all.
    ///
    /// `Unknown` returns `None`: an unrecognised map is exported alongside the
    /// others but is not guessed into the graph.
    pub const fn binding(self) -> Option<Binding> {
        match self {
            TextureRole::BaseColor => Some(Binding::BaseColor),
            TextureRole::Normal => Some(Binding::Normal),
            TextureRole::Roughness => Some(Binding::Roughness),
            TextureRole::Metallic => Some(Binding::Metallic),
            TextureRole::AmbientOcclusion => Some(Binding::AmbientOcclusion),
            TextureRole::Displacement => Some(Binding::Displacement),
            TextureRole::Opacity => Some(Binding::Alpha),
            TextureRole::Specular => Some(Binding::Specular),
            TextureRole::Glossiness => Some(Binding::InvertedRoughness),
            TextureRole::Emissive => Some(Binding::Emission),
            TextureRole::Unknown => None,
        }
    }
}

/// One texture map in a material, and where it lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureMap {
    /// The role the map's name put it in.
    pub role: TextureRole,
    /// How it wires into the material.
    pub binding: Binding,
    /// Its path within the export directory.
    pub target: std::path::PathBuf,
}

/// A material built from the maps found for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialSpec {
    /// The material's name.
    pub name: String,
    /// Its maps, in [`Binding::order`].
    pub maps: Vec<TextureMap>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_role_has_a_binding() {
        let roles = [
            TextureRole::BaseColor,
            TextureRole::Normal,
            TextureRole::Roughness,
            TextureRole::Metallic,
            TextureRole::AmbientOcclusion,
            TextureRole::Displacement,
            TextureRole::Opacity,
            TextureRole::Specular,
            TextureRole::Glossiness,
            TextureRole::Emissive,
        ];
        for role in roles {
            assert!(
                role.binding().is_some(),
                "{role:?} should wire into the graph"
            );
        }
    }

    #[test]
    fn an_unknown_role_is_not_wired() {
        assert_eq!(TextureRole::Unknown.binding(), None);
    }

    #[test]
    fn glossiness_inverts_rather_than_landing_on_roughness() {
        assert_eq!(
            TextureRole::Glossiness.binding(),
            Some(Binding::InvertedRoughness)
        );
        assert_ne!(
            TextureRole::Glossiness.binding(),
            TextureRole::Roughness.binding()
        );
    }

    #[test]
    fn base_colour_is_ordered_before_the_occlusion_that_multiplies_into_it() {
        assert!(Binding::BaseColor.order() < Binding::AmbientOcclusion.order());
    }

    #[test]
    fn binding_names_are_stable_and_distinct() {
        let all = [
            Binding::BaseColor,
            Binding::Roughness,
            Binding::Metallic,
            Binding::Specular,
            Binding::Alpha,
            Binding::Emission,
            Binding::Normal,
            Binding::InvertedRoughness,
            Binding::AmbientOcclusion,
            Binding::Displacement,
        ];
        let mut names: Vec<&str> = all.iter().map(|b| b.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
    }
}
