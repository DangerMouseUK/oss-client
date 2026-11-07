//! The built-in sandbox map, used when `--sandbox` gets no map file: a walled arena
//! around the map centre with pillars, rooms, asteroids and a safe zone.

use oss_sim::tile;

/// Arena half-size in tiles.
const HALF: i32 = 64;
const CENTER: i32 = 512;
const WALL: u8 = 1;

/// Builds the `.lvl` bytes (tile list only; the default tileset is used).
pub fn default_map() -> Vec<u8> {
    let mut tiles: Vec<(i32, i32, u8)> = Vec::new();
    let (lo, hi) = (CENTER - HALF, CENTER + HALF);
    // Outer wall.
    for i in lo..=hi {
        for (x, y) in [(i, lo), (i, hi), (lo, i), (hi, i)] {
            tiles.push((x, y, WALL));
        }
    }
    // A ring of 3×3 pillars.
    for (px, py) in [
        (-30, -30),
        (30, -30),
        (-30, 30),
        (30, 30),
        (0, -40),
        (0, 40),
        (-40, 0),
        (40, 0),
    ] {
        for dy in -1..=1 {
            for dx in -1..=1 {
                tiles.push((CENTER + px + dx, CENTER + py + dy, WALL));
            }
        }
    }
    // Two rooms with a gap, top-left and bottom-right.
    for (ox, oy) in [(-55, -55), (40, 40)] {
        for i in 0..15 {
            tiles.push((CENTER + ox + i, CENTER + oy, WALL));
            tiles.push((CENTER + ox, CENTER + oy + i, WALL));
            if i != 7 && i != 8 {
                tiles.push((CENTER + ox + i, CENTER + oy + 14, WALL));
                tiles.push((CENTER + ox + 14, CENTER + oy + i, WALL));
            }
        }
    }
    // A safe zone in the top-right corner.
    for y in lo + 2..lo + 8 {
        for x in hi - 7..hi - 1 {
            tiles.push((x, y, tile::SAFE));
        }
    }
    // Asteroids.
    for (x, y, id) in [
        (-15, 20, tile::ASTEROID_SMALL_1),
        (18, -12, tile::ASTEROID_LARGE),
        (25, 15, tile::ASTEROID_SMALL_2),
    ] {
        tiles.push((CENTER + x, CENTER + y, id));
    }
    let mut out = Vec::with_capacity(tiles.len() * 4);
    for (x, y, id) in tiles {
        out.extend_from_slice(&(x as u32 | ((y as u32) << 12) | ((id as u32) << 24)).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_has_room_to_spawn() {
        let level = oss_formats::Level::parse(&default_map()).unwrap();
        let (map, objects) =
            oss_sim::TileMap::from_entries(level.tiles.iter().map(|t| (t.x, t.y, t.tile)));
        assert_eq!(objects.len(), 3);
        assert!(map.is_solid(CENTER - HALF, CENTER));
        assert!(!map.box_collides(CENTER * 16 + 8, CENTER * 16 + 8, 14));
    }
}
