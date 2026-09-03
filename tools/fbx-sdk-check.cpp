// Imports an FBX with the Autodesk FBX SDK — the format's reference reader — and
// reports what it found as JSON (P2-10b). This is the one check assimp and
// Blender cannot stand in for: it is the exact library Maya/Max use, so an FBX
// it rejects is one Maya rejects.
//
// Build: tools/fbx-sdk-check.sh <file.fbx>
// Prints one JSON line; exits non-zero if the import fails.
#include <fbxsdk.h>
#include <cstdio>
#include <string>

static int countBones(FbxNode* node) {
    int n = 0;
    if (node->GetNodeAttribute() &&
        node->GetNodeAttribute()->GetAttributeType() == FbxNodeAttribute::eSkeleton) {
        n = 1;
    }
    for (int i = 0; i < node->GetChildCount(); ++i) n += countBones(node->GetChild(i));
    return n;
}

int main(int argc, char** argv) {
    if (argc < 2) { fprintf(stderr, "usage: %s <file.fbx>\n", argv[0]); return 2; }
    const char* path = argv[1];

    FbxManager* manager = FbxManager::Create();
    FbxIOSettings* ios = FbxIOSettings::Create(manager, IOSROOT);
    manager->SetIOSettings(ios);

    FbxImporter* importer = FbxImporter::Create(manager, "");
    if (!importer->Initialize(path, -1, manager->GetIOSettings())) {
        printf("{\"ok\":false,\"imported\":false,\"error\":\"%s\"}\n",
               importer->GetStatus().GetErrorString());
        manager->Destroy();
        return 1;
    }

    int major = 0, minor = 0, revision = 0;
    importer->GetFileVersion(major, minor, revision);
    FbxScene* scene = FbxScene::Create(manager, "scene");
    bool ok = importer->Import(scene);
    if (!ok) {
        printf("{\"ok\":false,\"imported\":false,\"file_version\":\"%d.%d.%d\",\"error\":\"%s\"}\n",
               major, minor, revision, importer->GetStatus().GetErrorString());
        importer->Destroy();
        manager->Destroy();
        return 1;
    }
    importer->Destroy();

    int meshes = 0, verts = 0;
    for (int i = 0; i < scene->GetSrcObjectCount<FbxMesh>(); ++i) {
        FbxMesh* mesh = scene->GetSrcObject<FbxMesh>(i);
        meshes++;
        verts += mesh->GetControlPointsCount();
    }
    int bones = scene->GetRootNode() ? countBones(scene->GetRootNode()) : 0;
    int stacks = scene->GetSrcObjectCount<FbxAnimStack>();

    // Frame range of the first stack, in seconds, so a clip's time axis is checked.
    double clipSeconds = 0.0;
    if (stacks > 0) {
        FbxAnimStack* stack = scene->GetSrcObject<FbxAnimStack>(0);
        FbxTimeSpan span = stack->GetLocalTimeSpan();
        clipSeconds = span.GetDuration().GetSecondDouble();
    }

    printf("{\"ok\":true,\"imported\":true,\"meshes\":%d,\"vertices\":%d,"
           "\"bones\":%d,\"anim_stacks\":%d,\"clip_seconds\":%.4f}\n",
           meshes, verts, bones, stacks, clipSeconds);

    manager->Destroy();
    return 0;
}
