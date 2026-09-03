#!/usr/bin/env python3
"""Splits a template skeleton into maximal parent->child chains.

A chain breaks where the skeleton branches: a bone with two or more children
ends its chain, and each child starts a new one. That is exactly the
decomposition `m2m-rig`'s template format wants, so this does the mechanical
part and leaves the judgement -- what each chain *is* -- to a person.

Reading a tree by eye is how the fox's ears got missed once already.

Usage: tools/glb-chains.py <file.glb>
"""
import json
import struct
import sys


def json_chunk(path):
    data = open(path, "rb").read()
    offset = 12
    while offset < len(data):
        length, kind = struct.unpack_from("<II", data, offset)
        if kind == 0x4E4F534A:
            return json.loads(data[offset + 8 : offset + 8 + length])
        offset += 8 + length
    raise SystemExit(f"{path}: no JSON chunk")


def main():
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    root = json_chunk(sys.argv[1])
    nodes = root["nodes"]
    joints = set(root["skins"][0]["joints"]) if root.get("skins") else set()

    children = {}
    parent = {}
    for index, node in enumerate(nodes):
        for child in node.get("children", []):
            parent[child] = index
            if child in joints and index in joints:
                children.setdefault(index, []).append(child)

    name = lambda i: nodes[i].get("name", f"<{i}>")
    # A chain starts at a joint whose parent is not a joint, or whose parent
    # has more than one joint child.
    starts = [
        j
        for j in joints
        if parent.get(j) not in joints or len(children.get(parent.get(j), [])) > 1
    ]

    chains = []
    for start in starts:
        run = [start]
        while len(children.get(run[-1], [])) == 1:
            run.append(children[run[-1]][0])
        chains.append(run)

    covered = sum(len(c) for c in chains)
    chains.sort(key=lambda c: name(c[0]))
    for run in chains:
        attaches = parent.get(run[0])
        under = name(attaches) if attaches in joints else "-"
        print(f"  [{len(run):>2}] under {under:<22} {' -> '.join(name(i) for i in run)}")
    print(f"\n{len(chains)} chains, {covered} bones, {len(joints)} joints")
    if covered != len(joints):
        raise SystemExit("chains do not cover every joint")


if __name__ == "__main__":
    main()
