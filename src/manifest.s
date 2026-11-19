# PE manifest resource used by Windows for layered child window support.
.section .rsrc,"dr"
.p2align 2
molap_resource_root:
    .long 0, 0, 0
    .short 0, 1
    .long 24, 0x80000018
    .long 0, 0, 0
    .short 0, 1
    .long 1, 0x80000030
    .long 0, 0, 0
    .short 0, 1
    .long 1033, 72
    .rva molap_manifest
    .long molap_manifest_end - molap_manifest, 0, 0
molap_manifest:
    .ascii "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
    .ascii "<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">"
    .ascii "<assemblyIdentity type=\"win32\" name=\"MoLap.Taskbar\" version=\"1.0.0.0\"/>"
    .ascii "<compatibility xmlns=\"urn:schemas-microsoft-com:compatibility.v1\"><application>"
    .ascii "<supportedOS Id=\"{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}\"/>"
    .ascii "<supportedOS Id=\"{1f676c76-80e1-4239-95bb-83d0f6d0da78}\"/>"
    .ascii "<supportedOS Id=\"{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}\"/>"
    .ascii "</application></compatibility></assembly>"
molap_manifest_end:
    .p2align 2
