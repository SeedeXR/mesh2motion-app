//! Prints the animation-related nodes of an FBX, with their child node names
//! and property types, so two files can be compared structurally.

use m2m_io::fbx::binary::{self, FbxNode, FbxProperty};

fn kind(p: &FbxProperty) -> String {
    match p {
        FbxProperty::Bool(_) => "Bool".into(),
        FbxProperty::I16(_) => "I16".into(),
        FbxProperty::I32(v) => format!("I32({v})"),
        FbxProperty::I64(v) => format!("I64({v})"),
        FbxProperty::F32(_) => "F32".into(),
        FbxProperty::F64(_) => "F64".into(),
        FbxProperty::Str(s) => format!(
            "Str({:?})",
            s.replace('\u{0}', "\\0").replace('\u{1}', "\\1")
        ),
        FbxProperty::Raw(b) => format!("Raw[{}]", b.len()),
        FbxProperty::BoolArray(a) => format!("BoolArray[{}]", a.len()),
        FbxProperty::I32Array(a) => format!("I32Array[{}]", a.len()),
        FbxProperty::I64Array(a) => format!("I64Array[{}]", a.len()),
        FbxProperty::F32Array(a) => format!("F32Array[{}]", a.len()),
        FbxProperty::F64Array(a) => format!("F64Array[{}]", a.len()),
    }
}

fn show(node: &FbxNode, depth: usize, limit: usize) {
    let pad = "  ".repeat(depth);
    let props: Vec<String> = node.properties.iter().map(kind).collect();
    println!("{pad}{} [{}]", node.name, props.join(", "));
    if depth < limit {
        for child in &node.children {
            show(child, depth + 1, limit);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: dump_anim <file.fbx>")?;
    let document = binary::parse(&std::fs::read(&path)?)?;

    for want in ["AnimationStack", "AnimationLayer", "AnimationCurveNode"] {
        if let Some(objects) = document.root("Objects") {
            if let Some(first) = objects.children_named(want).next() {
                println!("--- first {want}");
                show(first, 1, 3);
            } else {
                println!("--- no {want}");
            }
        }
    }
    if let Some(objects) = document.root("Objects") {
        if let Some(curve) = objects.children_named("AnimationCurve").next() {
            println!("--- first AnimationCurve");
            show(curve, 1, 2);
        }
    }
    for want in ["Definitions"] {
        match document.root(want) {
            Some(node) => {
                println!("--- {want}");
                show(node, 1, 3);
            }
            None => println!("--- no {want} root"),
        }
    }
    Ok(())
}
