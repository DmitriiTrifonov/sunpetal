//! Reader of the `sunpetal-ee` export (`out/export/v1`, K-234): the PS2 scene data the engine takes as data — cameras
//! (`cameras/<R>.json`: GPA paths, camera areas and points), collision and light (`scenes/<R>.json`: `hit`, `height`,
//! `light`, `fog`) and the DS-map ↔ PS2-scene table (`maps.tsv`). Formats: `sunpetal-ee/notes/formats/{camera,hit,light}.md`.
//! The files are not in the repository; the types are plain data, read from `serde_json::Value` (`serde_json`, already in the workspace).
use serde_json::Value;
use std::path::{Path, PathBuf};

type R<T> = Result<T, String>;

fn f(v: &Value, k: &str) -> R<f32> {
    v.get(k).and_then(Value::as_f64).map(|x| x as f32).ok_or_else(|| format!("no number `{}`", k))
}

fn u(v: &Value, k: &str) -> R<u32> {
    v.get(k).and_then(Value::as_u64).map(|x| x as u32).ok_or_else(|| format!("no integer `{}`", k))
}

fn arr<'a>(v: &'a Value, k: &str) -> R<&'a Vec<Value>> {
    v.get(k).and_then(Value::as_array).ok_or_else(|| format!("no list `{}`", k))
}

fn floats<const N: usize>(v: &Value) -> R<[f32; N]> {
    let a = v.as_array().filter(|a| a.len() >= N).ok_or_else(|| format!("a list of {} numbers", N))?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().ok_or("a number")? as f32;
    }
    Ok(out)
}

/// A key of a camera path (`GPA_KEYDATA`).
#[derive(Clone, Debug, PartialEq)]
pub struct CamKey {
    pub frame: f32,
    pub theta: f32,
    pub pos: [f32; 3],
    /// Rotation about x and y.
    pub rot: [f32; 2],
    pub phai: f32,
    pub param_t: f32,
    /// The spline's tangents.
    pub param: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamPath {
    pub flag: u32,
    pub last_frame: i32,
    pub keys: Vec<CamKey>,
}

/// The camera of a 3D scene record (`GPA_HEADER`).
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub id: u32,
    pub flag: u32,
    pub start_path: i32,
    pub scale: f32,
    pub target: [f32; 3],
    pub paths: Vec<CamPath>,
}

/// A camera area: where the player stands decides the mode and path of the camera.
#[derive(Clone, Debug, PartialEq)]
pub struct CamArea {
    pub index: u32,
    pub mode: u32,
    pub shape: u32,
    /// −1, 0 or 1 (the direction the path is walked).
    pub path_dir: i32,
    pub fields: Vec<f32>,
}

/// A fixed camera point: place and rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct CamPoint {
    pub pos: [f32; 3],
    pub rot: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneCamera {
    pub index: u32,
    pub record: u32,
    /// None: the record has no camera data (a scene without cuts).
    pub camera: Option<Camera>,
    pub areas: Vec<CamArea>,
    pub points: Vec<CamPoint>,
}

fn camera_path(p: &Value) -> R<CamPath> {
    let keys = arr(p, "keys")?
        .iter()
        .map(|k| {
            Ok(CamKey {
                frame: f(k, "frame")?,
                theta: f(k, "theta")?,
                pos: [f(k, "px")?, f(k, "py")?, f(k, "pz")?],
                rot: [f(k, "rx")?, f(k, "ry")?],
                phai: f(k, "phai")?,
                param_t: f(k, "param_t")?,
                param: [f(k, "param_a")?, f(k, "param_b")?, f(k, "param_c")?],
            })
        })
        .collect::<R<_>>()?;
    Ok(CamPath { flag: u(p, "flag")?, last_frame: p.get("lastframe").and_then(Value::as_i64).unwrap_or(0) as i32, keys })
}

/// The cameras of a scene from the text of `cameras/<R>.json`.
pub fn parse_cameras(text: &str) -> R<Vec<SceneCamera>> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    arr(&v, "scene3d")?
        .iter()
        .map(|s| {
            let camera = match s.get("camera").filter(|c| !c.is_null()) {
                None => None,
                Some(c) => {
                    // a record with no camera path (`npaths` 0) has no list
                    let paths = c.get("paths").and_then(Value::as_array).map_or(Ok(Vec::new()), |l| l.iter().map(camera_path).collect::<R<Vec<_>>>())?;
                    Some(Camera {
                        id: u(c, "id")?,
                        flag: u(c, "flag")?,
                        start_path: c.get("start_path").and_then(Value::as_i64).unwrap_or(0) as i32,
                        scale: f(c, "scale")?,
                        target: floats(c.get("target").ok_or("no target")?)?,
                        paths,
                    })
                }
            };
            let areas = arr(s, "camera_areas")?
                .iter()
                .map(|a| {
                    Ok(CamArea {
                        index: u(a, "index")?,
                        mode: u(a, "mode")?,
                        shape: u(a, "shape")?,
                        path_dir: a.get("path_dir").and_then(Value::as_i64).ok_or("no integer `path_dir`")? as i32,
                        fields: a.get("fields").and_then(Value::as_array).map(|x| x.iter().filter_map(|n| n.as_f64().map(|n| n as f32)).collect()).unwrap_or_default(),
                    })
                })
                .collect::<R<_>>()?;
            let points = arr(s, "camera_points")?
                .iter()
                .map(|p| Ok(CamPoint { pos: [f(p, "x")?, f(p, "y")?, f(p, "z")?], rot: [f(p, "rx")?, f(p, "ry")?, f(p, "rz")?] }))
                .collect::<R<_>>()?;
            Ok(SceneCamera { index: u(s, "index")?, record: u(s, "record")?, camera, areas, points })
        })
        .collect()
}

/// A walking-area shape on the floor plane (x, z), `plane` 1.
#[derive(Clone, Debug, PartialEq)]
pub enum HitShape {
    Rect {
        x0: f32,
        z0: f32,
        dx: f32,
        dz: f32,
    },
    Ellipse {
        cx: f32,
        cz: f32,
        rx: f32,
        rz: f32,
    },
    /// A closed polygon.
    Polygon(Vec<[f32; 2]>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub enabled: bool,
    pub shape: HitShape,
}

/// A triangle of the floor height (x, y, z, w per corner).
#[derive(Clone, Debug, PartialEq)]
pub struct HeightTri {
    pub enabled: bool,
    pub points: Vec<[f32; 4]>,
}

/// The collision of one 3D scene record.
#[derive(Clone, Debug, PartialEq)]
pub struct Area3d {
    pub index: u32,
    pub record: u32,
    pub hit: Vec<Hit>,
    pub height: Vec<HeightTri>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub ambient: [f32; 3],
    pub dir: [f32; 3],
    pub color: [f32; 3],
    /// The game's `GetLightMatrix` result (columns are the direction).
    pub matrix: [[f32; 4]; 4],
    /// `default`: the scene has no light of its own.
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fog {
    pub enabled: bool,
    pub near: f32,
    pub far: f32,
    pub power: f32,
    pub color: [u8; 3],
}

/// What `scenes/<R>.json` holds for the engine: collision per record, light and fog of the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneData {
    pub scene: String,
    pub areas: Vec<Area3d>,
    pub light: Option<Light>,
    pub fog: Option<Fog>,
}

fn hit(h: &Value) -> R<Hit> {
    let shape = match u(h, "shape")? {
        3 => HitShape::Rect { x0: f(h, "x0")?, z0: f(h, "z0")?, dx: f(h, "dx")?, dz: f(h, "dz")? },
        2 => HitShape::Ellipse { cx: f(h, "cx")?, cz: f(h, "cz")?, rx: f(h, "rx")?, rz: f(h, "rz")? },
        1 => HitShape::Polygon(arr(h, "points")?.iter().map(floats::<2>).collect::<R<_>>()?),
        n => return Err(format!("hit shape {}", n)),
    };
    Ok(Hit { enabled: u(h, "enabled")? != 0, shape })
}

/// The scene data from the text of `scenes/<R>.json`.
pub fn parse_scene(text: &str) -> R<SceneData> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let areas = arr(&v, "scene3d")?
        .iter()
        .map(|a| {
            let hits = a.get("hit").and_then(Value::as_array).map_or(Ok(Vec::new()), |l| l.iter().map(hit).collect::<R<Vec<_>>>())?;
            let height = a.get("height").and_then(Value::as_array).map_or(Ok(Vec::new()), |l| {
                l.iter().map(|t| Ok(HeightTri { enabled: u(t, "enabled")? != 0, points: arr(t, "points")?.iter().map(floats::<4>).collect::<R<_>>()? })).collect::<R<Vec<_>>>()
            })?;
            Ok(Area3d { index: u(a, "index")?, record: u(a, "record")?, hit: hits, height })
        })
        .collect::<R<_>>()?;
    let light = match v.get("light").filter(|l| !l.is_null()) {
        Some(l) => {
            let first = arr(l, "lights")?.first().ok_or("no light")?;
            let m = arr(l, "matrix")?;
            let mut matrix = [[0.0; 4]; 4];
            for (row, r) in matrix.iter_mut().zip(m) {
                *row = floats(r)?;
            }
            Some(Light {
                ambient: floats(l.get("ambient").ok_or("no ambient")?)?,
                dir: floats(first.get("dir").ok_or("no dir")?)?,
                color: floats(first.get("color").ok_or("no color")?)?,
                matrix,
                source: l.get("source").and_then(Value::as_str).unwrap_or("").to_string(),
            })
        }
        None => None,
    };
    let fog = match v.get("fog").filter(|l| !l.is_null()) {
        Some(g) => {
            let c: [f32; 3] = floats(g.get("color").ok_or("no fog color")?)?;
            Some(Fog {
                enabled: g.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                near: f(g, "near")?,
                far: f(g, "far")?,
                power: f(g, "power")?,
                color: c.map(|x| x as u8),
            })
        }
        None => None,
    };
    Ok(SceneData { scene: v.get("scene").and_then(Value::as_str).unwrap_or("").to_string(), areas, light, fog })
}

/// A row of `maps.tsv`: the DS area and the PS2 scene that holds its model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapRow {
    /// The DS map number (`A16` is 16).
    pub map: u32,
    pub ds_model: String,
    pub ps2_scene: String,
    pub ps2_model: String,
    pub status: String,
    pub confidence: String,
    pub chapter: String,
}

pub fn parse_maps(text: &str) -> R<Vec<MapRow>> {
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().ok_or("empty maps.tsv")?.split('\t').collect();
    let col = |n: &str| head.iter().position(|h| *h == n).ok_or_else(|| format!("maps.tsv has no column {}", n));
    let (a, m, s, p, st, c, ch) = (col("ds_area")?, col("ds_model")?, col("ps2_scene")?, col("ps2_model")?, col("status")?, col("confidence")?, col("chapter")?);
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let x: Vec<&str> = l.split('\t').collect();
            let get = |i: usize| x.get(i).map(|s| s.to_string()).ok_or_else(|| format!("short row: {}", l));
            let map = get(a)?.strip_prefix('A').and_then(|n| n.parse().ok()).ok_or_else(|| format!("ds_area in {}", l))?;
            Ok(MapRow { map, ds_model: get(m)?, ps2_scene: get(s)?, ps2_model: get(p)?, status: get(st)?, confidence: get(c)?, chapter: get(ch)? })
        })
        .collect()
}

/// The export folder (`out/export/v1`).
pub struct Export {
    root: PathBuf,
}

impl Export {
    pub fn open(root: impl Into<PathBuf>) -> R<Export> {
        let root = root.into();
        if root.join("manifest.json").is_file() {
            Ok(Export { root })
        } else {
            Err(format!("{}: no manifest.json, not an export", root.display()))
        }
    }

    fn read(&self, rel: &str) -> R<String> {
        std::fs::read_to_string(self.root.join(rel)).map_err(|e| format!("{}: {}", rel, e))
    }

    pub fn cameras(&self, scene: &str) -> R<Vec<SceneCamera>> {
        parse_cameras(&self.read(&format!("cameras/{}.json", scene))?)
    }

    pub fn scene(&self, scene: &str) -> R<SceneData> {
        parse_scene(&self.read(&format!("scenes/{}.json", scene))?)
    }

    pub fn maps(&self) -> R<Vec<MapRow>> {
        parse_maps(&self.read("maps.tsv")?)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMERAS: &str = r#"{"scene":"R01","scene3d":[{"index":0,"record":1,"errors":[],
      "camera":{"id":0,"flag":256,"start_path":1,"scale":1.0,"target":[0,0,0],
        "paths":[{"flag":1,"lastframe":30,"nkeys":2,"keys":[
          {"frame":0,"theta":0,"px":-172,"py":202,"pz":144,"rx":0,"ry":-3.14,"phai":0,"param_t":0,"param_a":0,"param_b":0,"param_c":0},
          {"frame":30,"theta":0,"px":-181,"py":203,"pz":90,"rx":0,"ry":-3.0,"phai":0,"param_t":1,"param_a":0,"param_b":0,"param_c":0}]}]},
      "camera_areas":[{"index":1,"mode":2,"npoints":0,"path_dir":0,"shape":3,"fields":[117.5,-188.5,113.5,81.5]}],
      "camera_points":[{"x":-163.25,"y":186.5,"z":145.5,"rx":0.25,"ry":0.96,"rz":0}]}]}"#;

    const SCENE: &str = r#"{"scene":"R01","scene3d":[{"index":0,"record":1,
      "hit":[{"enabled":0,"shape":3,"x0":-221.5,"z0":200.5,"dx":123.0,"dz":29.5},
             {"enabled":1,"shape":2,"cx":-116.5,"cz":86.5,"rx":11.0,"rz":12.0},
             {"enabled":0,"shape":1,"points":[[-222.5,-174.0],[-174.5,-172.5],[-221.5,-131.5]]}],
      "height":[{"enabled":0,"points":[[-70.5,186,265.5,1],[-274.5,186,263.5,1],[-180.5,186,141.5,1]]}]}],
      "light":{"ambient":[0,0,0],"source":"default","lights":[{"dir":[0.5,1.5,1.0],"color":[1,1,1]}],
        "matrix":[[0.5,0.5,0.5,0],[1.5,1.5,1.5,0],[1,1,1,0],[0,0,0,1]]},
      "fog":{"enabled":false,"near":2500,"far":10000,"power":1,"color":[192,192,192]}}"#;

    #[test]
    fn a_scene_camera_reads() {
        let c = parse_cameras(CAMERAS).unwrap();
        assert_eq!(c.len(), 1);
        let cam = c[0].camera.as_ref().unwrap();
        assert_eq!((cam.flag, cam.paths.len(), cam.paths[0].keys.len(), cam.paths[0].last_frame), (256, 1, 2, 30));
        assert_eq!(cam.paths[0].keys[1].pos, [-181.0, 203.0, 90.0]);
        assert_eq!((c[0].areas[0].mode, c[0].areas[0].fields.len(), c[0].points[0].pos), (2, 4, [-163.25, 186.5, 145.5]));
    }

    #[test]
    fn collision_light_and_fog_read() {
        let s = parse_scene(SCENE).unwrap();
        let a = &s.areas[0];
        assert!(matches!(a.hit[0].shape, HitShape::Rect { dx, .. } if dx == 123.0));
        assert!(a.hit[1].enabled && matches!(a.hit[1].shape, HitShape::Ellipse { rx, .. } if rx == 11.0));
        assert!(matches!(&a.hit[2].shape, HitShape::Polygon(p) if p.len() == 3));
        assert_eq!(a.height[0].points[2], [-180.5, 186.0, 141.5, 1.0]);
        let l = s.light.unwrap();
        assert_eq!((l.dir, l.source.as_str(), l.matrix[1][0]), ([0.5, 1.5, 1.0], "default", 1.5));
        let g = s.fog.unwrap();
        assert_eq!((g.enabled, g.near, g.far, g.color), (false, 2500.0, 10000.0, [192; 3]));
    }

    #[test]
    fn a_bad_shape_is_an_error() {
        assert!(parse_scene(r#"{"scene3d":[{"index":0,"record":1,"hit":[{"enabled":0,"shape":9}]}]}"#).is_err());
    }

    #[test]
    fn the_map_table_reads() {
        let t = "ds_area\tds_model\tps2_scene\tps2_model\tstatus\tconfidence\tchapter\nA16\tno16_hatake\tR15\tf13a780221\tswap\tmedium\tC14\n";
        let m = parse_maps(t).unwrap();
        assert_eq!((m[0].map, m[0].ps2_scene.as_str(), m[0].confidence.as_str()), (16, "R15", "medium"));
        assert!(parse_maps("a\tb\n").is_err());
    }

    /// With the real export (`SUNPETAL_EE_EXPORT=…/out/export/v1`): every scene reads, and the numbers of the notes hold.
    #[test]
    fn the_real_export_reads() {
        let Some(root) = std::env::var_os("SUNPETAL_EE_EXPORT") else { return };
        let e = Export::open(root).unwrap();
        let maps = e.maps().unwrap();
        assert!(maps.iter().any(|m| m.map == 85 && m.ps2_scene == "OP"));
        let (mut rect, mut ell, mut poly, mut tri) = (0, 0, 0, 0);
        for scene in std::fs::read_dir(e.root().join("scenes")).unwrap().filter_map(|x| x.ok()).filter_map(|x| x.path().file_stem().map(|s| s.to_string_lossy().into_owned())) {
            let s = e.scene(&scene).unwrap();
            e.cameras(&scene).unwrap();
            for a in &s.areas {
                tri += a.height.len();
                for h in &a.hit {
                    match h.shape {
                        HitShape::Rect { .. } => rect += 1,
                        HitShape::Ellipse { .. } => ell += 1,
                        HitShape::Polygon(_) => poly += 1,
                    }
                }
            }
        }
        // `notes/formats/hit.md`: rect 4933, ellipse 1970, line 3477, height triangles 10207
        assert_eq!((rect, ell, poly, tri), (4933, 1970, 3477, 10207));
    }
}
