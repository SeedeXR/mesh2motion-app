//! Prints the connections that touch animation objects, so two files can be
//! compared by how their animation graph is wired rather than by its contents.

use m2m_io::fbx::binary::{self, FbxProperty};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: dump_conn <file.fbx>")?;
    let document = binary::parse(&std::fs::read(&path)?)?;

    // id -> (node name, object name)
    let mut objects: HashMap<i64, (String, String)> = HashMap::new();
    for root in document.roots.iter().filter(|n| n.name == "Objects") {
        for child in &root.children {
            if let (Some(FbxProperty::I64(id)), Some(FbxProperty::Str(name))) =
                (child.properties.first(), child.properties.get(1))
            {
                objects.insert(
                    *id,
                    (
                        child.name.clone(),
                        name.replace('\u{0}', "\\0").replace('\u{1}', "\\1"),
                    ),
                );
            }
        }
    }
    let describe = |id: i64| -> String {
        if id == 0 {
            return "ROOT(0)".into();
        }
        match objects.get(&id) {
            Some((kind, name)) => format!("{kind}({name})"),
            None => format!("?({id})"),
        }
    };
    let is_anim = |id: i64| {
        objects
            .get(&id)
            .is_some_and(|(kind, _)| kind.starts_with("Animation"))
    };

    let mut seen: HashMap<String, usize> = HashMap::new();
    for root in document.roots.iter().filter(|n| n.name == "Connections") {
        for c in &root.children {
            let (
                Some(FbxProperty::Str(kind)),
                Some(FbxProperty::I64(src)),
                Some(FbxProperty::I64(dst)),
            ) = (
                c.properties.first(),
                c.properties.get(1),
                c.properties.get(2),
            )
            else {
                continue;
            };
            if !is_anim(*src) && !is_anim(*dst) {
                continue;
            }
            let prop = match c.properties.get(3) {
                Some(FbxProperty::Str(p)) => format!(" \"{p}\""),
                _ => String::new(),
            };
            let shape = format!(
                "{kind}: {} -> {}{prop}",
                describe(*src).split('(').next().unwrap_or(""),
                describe(*dst).split('(').next().unwrap_or(""),
            );
            *seen.entry(shape).or_default() += 1;
        }
    }
    let mut shapes: Vec<_> = seen.into_iter().collect();
    shapes.sort();
    for (shape, count) in shapes {
        println!("  {count:>5} x  {shape}");
    }
    Ok(())
}
