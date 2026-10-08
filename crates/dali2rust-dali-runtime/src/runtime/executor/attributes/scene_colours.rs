use super::*;
use dali2rust_contracts::msg::DaliAttributeReadChunk as Chunk;

pub(super) fn read_scene_colours(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Vec<dali2rust_contracts::msg::DaliAttributeReadChunk>, SemanticDaliError> {
    let mut chunks = Vec::with_capacity(16);
    for scene in 0..16u8 {
        chunks.push(read_one_scene_colour(
            controller,
            address,
            scene,
            content_confirm,
        )?);
    }
    Ok(chunks)
}

pub fn read_scene_colour_readback(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    scene: u8,
) -> Result<dali2rust_contracts::msg::DaliAttributeReadChunk, SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    read_one_scene_colour(controller, address, scene, ContentConfirmPolicy::default())
}

// IEC 62386-209 §9.11.5, §9.12.6
fn read_one_scene_colour(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    scene: u8,
    content_confirm: ContentConfirmPolicy,
) -> Result<dali2rust_contracts::msg::DaliAttributeReadChunk, SemanticDaliError> {
    controller.unit_exempt(|controller| {
        let level = send_standard_query(
            controller,
            address,
            StandardCommand::QuerySceneLevel { scene },
        )?;
        if level.is_none() {
            return Ok(Chunk::SceneColour {
                scene,
                level: None,
                colour_type: None,
                values: [None; 6],
            });
        }
        let colour_type = confirm_observed_query(controller, content_confirm, |c| {
            dt8_narrow_value_sample(c, address, DT8_COLOUR_VALUE_REPORT_COLOUR_TYPE)
        })?;
        let values = read_report_values(controller, address, colour_type, content_confirm)?;
        Ok(Chunk::SceneColour {
            scene,
            level,
            colour_type,
            values,
        })
    })
}

fn read_report_values(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    colour_type: Option<u8>,
    content_confirm: ContentConfirmPolicy,
) -> Result<[Option<u16>; 6], SemanticDaliError> {
    let mut values = [None; 6];
    match colour_type {
        Some(DT8_COLOUR_TYPE_TC) => {
            values[0] = read_dt8_color_value_u16(
                controller,
                address,
                DT8_COLOUR_VALUE_REPORT_TC,
                content_confirm,
            )?;
        }
        Some(DT8_COLOUR_TYPE_XY) => {
            for (slot, id) in [DT8_COLOUR_VALUE_REPORT_X, DT8_COLOUR_VALUE_REPORT_Y]
                .into_iter()
                .enumerate()
            {
                values[slot] = read_dt8_color_value_u16(controller, address, id, content_confirm)?;
            }
        }
        Some(DT8_COLOUR_TYPE_RGBWAF) => {
            for (slot, value) in values.iter_mut().enumerate() {
                *value = read_dt8_color_value_u8(
                    controller,
                    address,
                    DT8_COLOUR_VALUE_REPORT_RED + slot as u8,
                    content_confirm,
                )?
                .map(u16::from);
            }
        }
        _ => {}
    }
    Ok(values)
}
