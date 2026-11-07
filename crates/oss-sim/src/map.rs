//! Tile map used for collision. 1024×1024 tiles of 16×16 pixels; anything outside the
//! map counts as solid wall.

/// Map width/height in tiles.
pub const MAP_TILES: usize = 1024;

/// Well-known tile ids.
pub mod tile {
    pub const EMPTY: u8 = 0;
    pub const FIRST_DOOR: u8 = 162;
    pub const LAST_DOOR: u8 = 169;
    pub const FLAG: u8 = 170;
    pub const SAFE: u8 = 171;
    pub const GOAL: u8 = 172;
    pub const FIRST_FLY_OVER: u8 = 173;
    pub const LAST_FLY_OVER: u8 = 175;
    pub const FIRST_FLY_UNDER: u8 = 176;
    pub const LAST_FLY_UNDER: u8 = 190;
    pub const ASTEROID_SMALL_1: u8 = 216;
    pub const ASTEROID_LARGE: u8 = 217;
    pub const ASTEROID_SMALL_2: u8 = 218;
    pub const SPACE_STATION: u8 = 219;
    pub const WORMHOLE: u8 = 220;
    pub const BRICK: u8 = 250;
    /// Returned for coordinates outside the map.
    pub const OUTSIDE: u8 = 20;
}

/// Whether a tile id blocks ships and weapons.
pub const fn is_solid_id(id: u8) -> bool {
    match id {
        0 => false,
        1..=169 => true, // walls (1..=161) and closed doors (162..=169)
        tile::WORMHOLE => false,
        192..=240 => true,
        242..=252 => true,
        _ => false,
    }
}

/// Size in tiles of map objects that occupy more than one tile. The `.lvl` file stores
/// only the top-left tile; the rest of the square is filled with the same id.
pub const fn object_size(id: u8) -> u8 {
    match id {
        tile::ASTEROID_LARGE => 2,
        tile::SPACE_STATION => 6,
        tile::WORMHOLE => 5,
        _ => 1,
    }
}

/// Whether a tile id is drawn as an animated object rather than from the tileset.
pub const fn is_animated_object(id: u8) -> bool {
    matches!(
        id,
        tile::FLAG
            | tile::GOAL
            | tile::ASTEROID_SMALL_1
            | tile::ASTEROID_LARGE
            | tile::ASTEROID_SMALL_2
            | tile::SPACE_STATION
            | tile::WORMHOLE
    )
}

/// An animated map object's top-left tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapObject {
    pub x: u16,
    pub y: u16,
    pub id: u8,
}

/// A 1024×1024 tile grid.
#[derive(Clone, PartialEq, Eq)]
pub struct TileMap {
    tiles: Box<[u8]>,
}

impl std::fmt::Debug for TileMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileMap").finish_non_exhaustive()
    }
}

impl Default for TileMap {
    fn default() -> Self {
        Self::empty()
    }
}

impl TileMap {
    /// An all-empty map.
    pub fn empty() -> Self {
        Self {
            tiles: vec![0u8; MAP_TILES * MAP_TILES].into_boxed_slice(),
        }
    }

    /// Builds a map from row-major cells (`y * 1024 + x`). Short input is padded with
    /// empty tiles; extra input is ignored.
    pub fn from_cells(cells: &[u8]) -> Self {
        let mut m = Self::empty();
        let n = cells.len().min(MAP_TILES * MAP_TILES);
        m.tiles[..n].copy_from_slice(&cells[..n]);
        m
    }

    /// Builds a map from `.lvl` tile entries `(x, y, id)`, expanding multi-tile objects
    /// and collecting animated objects (asteroids, stations, wormholes, goals, flags).
    pub fn from_entries(
        entries: impl IntoIterator<Item = (u16, u16, u8)>,
    ) -> (Self, Vec<MapObject>) {
        let mut m = Self::empty();
        let mut objects = Vec::new();
        for (x, y, id) in entries {
            if x as usize >= MAP_TILES || y as usize >= MAP_TILES {
                continue;
            }
            if is_animated_object(id) {
                objects.push(MapObject { x, y, id });
            }
            let size = object_size(id) as i32;
            for dy in 0..size {
                for dx in 0..size {
                    m.set(x as i32 + dx, y as i32 + dy, id);
                }
            }
        }
        (m, objects)
    }

    pub fn cells(&self) -> &[u8] {
        &self.tiles
    }

    /// Tile at (x, y); [`tile::OUTSIDE`] (a wall) outside the map.
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= MAP_TILES as i32 || y >= MAP_TILES as i32 {
            return tile::OUTSIDE;
        }
        self.tiles[y as usize * MAP_TILES + x as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, id: u8) {
        if x >= 0 && y >= 0 && x < MAP_TILES as i32 && y < MAP_TILES as i32 {
            self.tiles[y as usize * MAP_TILES + x as usize] = id;
        }
    }

    pub fn is_solid(&self, x: i32, y: i32) -> bool {
        is_solid_id(self.get(x, y))
    }

    /// Tile under a pixel coordinate.
    pub fn tile_at_pixel(&self, px: i32, py: i32) -> u8 {
        self.get(px.div_euclid(16), py.div_euclid(16))
    }

    /// Whether a square of half-size `radius` pixels centred on pixel (px, py) touches
    /// any solid tile. The box is inclusive on both edges, matching Continuum.
    pub fn box_collides(&self, px: i32, py: i32, radius: i32) -> bool {
        let (x0, x1) = ((px - radius).div_euclid(16), (px + radius).div_euclid(16));
        let (y0, y1) = ((py - radius).div_euclid(16), (py + radius).div_euclid(16));
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                if self.is_solid(tx, ty) {
                    return true;
                }
            }
        }
        false
    }

    /// Finds the nearest free spot (in pixels) for a ship of `radius` around a tile,
    /// searching outward in square rings up to `max_rings` tiles away.
    pub fn find_free_spot(
        &self,
        tx: i32,
        ty: i32,
        radius: i32,
        max_rings: i32,
    ) -> Option<(i32, i32)> {
        for ring in 0..=max_rings {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (px, py) = ((tx + dx) * 16 + 8, (ty + dy) * 16 + 8);
                    if !self.box_collides(px, py, radius) {
                        return Some((px, py));
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solidity_table() {
        assert!(!is_solid_id(0));
        assert!(is_solid_id(1));
        assert!(is_solid_id(161));
        assert!(is_solid_id(165));
        assert!(!is_solid_id(tile::SAFE));
        assert!(!is_solid_id(180));
        assert!(is_solid_id(tile::ASTEROID_LARGE));
        assert!(!is_solid_id(tile::WORMHOLE));
        assert!(!is_solid_id(241));
        assert!(is_solid_id(tile::BRICK));
        assert!(!is_solid_id(255));
    }

    #[test]
    fn objects_expand() {
        let (m, objs) =
            TileMap::from_entries([(10, 10, tile::SPACE_STATION), (1, 1, 5), (2000, 0, 1)]);
        assert_eq!(
            objs,
            vec![MapObject {
                x: 10,
                y: 10,
                id: tile::SPACE_STATION
            }]
        );
        assert_eq!(m.get(15, 15), tile::SPACE_STATION);
        assert_eq!(m.get(16, 15), 0);
        assert_eq!(m.get(1, 1), 5);
    }

    #[test]
    fn outside_is_wall_and_free_spot() {
        let mut m = TileMap::empty();
        assert!(m.is_solid(-1, 0));
        assert!(m.is_solid(0, 1024));
        m.set(10, 10, 1);
        assert!(m.box_collides(10 * 16 + 8, 10 * 16 + 8, 14));
        let (px, py) = m.find_free_spot(10, 10, 14, 5).unwrap();
        assert!(!m.box_collides(px, py, 14));
    }
}
