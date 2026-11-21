use pumpkin_data::{
    structures::{Structure, StructureKeys},
    tag::{RegistryKey, get_tag_ids},
};

use crate::{
    biome::BiomeSupplier,
    generation::{
        biome_coords,
        noise::router::multi_noise_sampler::MultiNoiseSampler,
        structure::structures::{
            StructureGenerator, StructureGeneratorContext, StructurePosition,
            buried_treasure::BuriedTreasureGenerator,
            desert_pyramid::DesertPyramidGenerator,
            end_city::EndCityGenerator,
            igloo::IglooGenerator,
            jigsaw::JigsawGenerator,
            jungle_temple::JungleTempleGenerator,
            mansion::MansionGenerator,
            mineshaft::MineshaftGenerator,
            nether_fortress::NetherFortressGenerator,
            nether_fossil::NetherFossilGenerator,
            ocean_monument::{OceanMonumentGenerator, has_valid_biomes},
            ocean_ruin::OceanRuinGenerator,
            ruined_portal::RuinedPortalGenerator,
            shipwreck::ShipwreckGenerator,
            stronghold::StrongholdGenerator,
            swamp_hut::SwampHutGenerator,
        },
    },
};

pub(crate) mod height_sampler;
pub mod piece;
pub mod placement;
pub mod shiftable_piece;
pub mod structures;
pub mod template;

/// Creates a structure position by dispatching to the appropriate generator.
/// Does NOT perform biome validation — callers that require it should add
/// their own check after calling this function.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn generate_structure_position(
    key: &StructureKeys,
    structure: &Structure,
    context: StructureGeneratorContext<'_>,
) -> Option<StructurePosition> {
    generate_structure_position_checked(*key, structure, context, |_| true)
}

#[must_use]
#[allow(clippy::too_many_lines)]
fn generate_structure_position_checked(
    key: StructureKeys,
    structure: &Structure,
    context: StructureGeneratorContext<'_>,
    is_valid_biome: impl FnOnce(pumpkin_util::math::position::BlockPos) -> bool,
) -> Option<StructurePosition> {
    match key {
        StructureKeys::BuriedTreasure => {
            BuriedTreasureGenerator::get_structure_position(&BuriedTreasureGenerator, context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
        StructureKeys::SwampHut => {
            SwampHutGenerator::get_structure_position(&SwampHutGenerator, context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
        StructureKeys::Stronghold => {
            StrongholdGenerator::get_structure_position(&StrongholdGenerator, context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
        StructureKeys::Fortress => {
            NetherFortressGenerator::get_structure_position(&NetherFortressGenerator, context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
        StructureKeys::NetherFossil => {
            NetherFossilGenerator::get_structure_position(&NetherFossilGenerator, context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
        StructureKeys::Igloo => IglooGenerator::get_structure_position(&IglooGenerator, context)
            .filter(|pos| is_valid_biome(pos.start_pos)),
        StructureKeys::DesertPyramid => DesertPyramidGenerator
            .get_structure_position(context)
            .filter(|pos| is_valid_biome(pos.start_pos)),
        StructureKeys::JunglePyramid => JungleTempleGenerator
            .get_structure_position(context)
            .filter(|pos| is_valid_biome(pos.start_pos)),
        StructureKeys::VillagePlains
        | StructureKeys::VillageDesert
        | StructureKeys::VillageSavanna
        | StructureKeys::VillageSnowy
        | StructureKeys::VillageTaiga
        | StructureKeys::AbandonedCampBambooJungle
        | StructureKeys::AbandonedCampBirchForest
        | StructureKeys::AbandonedCampCherryGrove
        | StructureKeys::AbandonedCampDappledForest
        | StructureKeys::AbandonedCampFlowerForest
        | StructureKeys::AbandonedCampForest
        | StructureKeys::AbandonedCampMeadow
        | StructureKeys::AbandonedCampOldGrowthBirchForest
        | StructureKeys::AbandonedCampOldGrowthPineTaiga
        | StructureKeys::AbandonedCampOldGrowthSpruceTaiga
        | StructureKeys::AbandonedCampPaleGarden
        | StructureKeys::AbandonedCampSavanna
        | StructureKeys::AbandonedCampSnowyTaiga
        | StructureKeys::AbandonedCampSparseJungle
        | StructureKeys::AbandonedCampSwamp
        | StructureKeys::AbandonedCampTaiga
        | StructureKeys::AbandonedCampWindsweptForest
        | StructureKeys::AbandonedCampWoodedBadlands
        | StructureKeys::AncientCity
        | StructureKeys::BastionRemnant
        | StructureKeys::PillagerOutpost
        | StructureKeys::TrailRuins
        | StructureKeys::TrialChambers => {
            let (Some(start_pool), Some(size)) = (structure.start_pool, structure.size) else {
                return None;
            };
            let mut generator =
                JigsawGenerator::new(start_pool, size).with_pool_aliases(structure.pool_aliases);
            if structure.use_expansion_hack.unwrap_or(false) {
                generator = generator.with_expansion_hack(true);
            }
            if let Some(start_jigsaw_name) = structure.start_jigsaw_name {
                generator = generator.with_start_jigsaw(start_jigsaw_name);
            }
            generator.get_structure_position_checked(context, is_valid_biome)
        }
        StructureKeys::Shipwreck | StructureKeys::ShipwreckBeached => {
            let generator = ShipwreckGenerator {
                is_beached: structure
                    .is_beached
                    .unwrap_or(key == StructureKeys::ShipwreckBeached),
            };
            generator
                .get_structure_position(context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
        StructureKeys::RuinedPortal
        | StructureKeys::RuinedPortalDesert
        | StructureKeys::RuinedPortalJungle
        | StructureKeys::RuinedPortalSwamp
        | StructureKeys::RuinedPortalMountain
        | StructureKeys::RuinedPortalOcean
        | StructureKeys::RuinedPortalNether => {
            let generator = RuinedPortalGenerator { variant: key };
            generator
                .get_structure_position(context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
        StructureKeys::OceanRuinCold | StructureKeys::OceanRuinWarm => {
            let generator = OceanRuinGenerator {
                is_warm: structure
                    .biome_temp
                    .map_or(key == StructureKeys::OceanRuinWarm, |t| t == "warm"),
            };
            generator
                .get_structure_position(context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
        StructureKeys::EndCity => EndCityGenerator
            .get_structure_position(context)
            .filter(|pos| is_valid_biome(pos.start_pos)),
        StructureKeys::Mansion => MansionGenerator
            .get_structure_position(context)
            .filter(|pos| is_valid_biome(pos.start_pos)),
        StructureKeys::Monument => OceanMonumentGenerator
            .get_structure_position(context)
            .filter(|pos| is_valid_biome(pos.start_pos)),
        StructureKeys::Mineshaft | StructureKeys::MineshaftMesa => {
            let generator = MineshaftGenerator {
                is_mesa: structure
                    .mineshaft_type
                    .map_or(key == StructureKeys::MineshaftMesa, |t| t == "mesa"),
            };
            generator
                .get_structure_position(context)
                .filter(|pos| is_valid_biome(pos.start_pos))
        }
    }
}

#[must_use]
pub fn lazily_generate_structure(
    key: &StructureKeys,
    structure: &Structure,
    mut context: StructureGeneratorContext, // Replaces 5 separate arguments!
    biome_supplier: &dyn BiomeSupplier,
    multi_noise_sampler: &mut MultiNoiseSampler,
) -> Option<StructurePosition> {
    let biomes = get_tag_ids(
        RegistryKey::WorldgenBiome,
        structure
            .biomes
            .strip_prefix('#')
            .unwrap_or(structure.biomes),
    )?;
    if *key == StructureKeys::Monument {
        let center_x = crate::generation::positions::chunk_pos::get_center_x(context.chunk_x);
        let center_z = crate::generation::positions::chunk_pos::get_center_z(context.chunk_z);
        let start_y = context
            .height_sampler
            .as_deref_mut()
            .map_or(context.sea_level, |sampler| {
                sampler.estimate_ocean_floor_height(center_x, center_z)
            });
        if !has_valid_biomes(
            biome_supplier,
            multi_noise_sampler,
            context.chunk_x,
            context.chunk_z,
            context.sea_level,
            start_y,
        ) {
            return None;
        }
    }

    generate_structure_position_checked(*key, structure, context, |start_pos| {
        let biome_x = biome_coords::from_block(start_pos.0.x);
        let biome_y = biome_coords::from_block(start_pos.0.y);
        let biome_z = biome_coords::from_block(start_pos.0.z);

        let biome = biome_supplier.biome(biome_x, biome_y, biome_z, multi_noise_sampler);
        biomes.contains(&(biome.id as u16))
    })
}
