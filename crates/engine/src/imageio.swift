// LightCraft's ImageIO helper (macOS): image formats only macOS's own encoders write.
// Built once with swiftc into LightCraft's support folder (see imageio.rs). Original work.
//
//   lc-imageio gainmap-avif <in.jpg> <out.avif> <quality 1-100>
//     An ISO 21496-1 gain-map JPEG (SDR base + gain map, as LightCraft writes it) re-encoded as an
//     AVIF with the same base, metadata and gain map (AVIF 'tmap' item). Exit 2: the input has no
//     ISO gain map; 3: ImageIO can't write AVIF here; 1: anything else.
import Foundation
import ImageIO

func fail(_ code: Int32, _ msg: String) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(code)
}

let args = CommandLine.arguments
guard args.count >= 5, args[1] == "gainmap-avif" else {
    fail(1, "usage: lc-imageio gainmap-avif <in.jpg> <out.avif> <quality>")
}
let quality = max(1.0, min(100.0, Double(args[4]) ?? 90.0)) / 100.0
guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[2]) as CFURL, nil), CGImageSourceGetCount(src) > 0 else {
    fail(1, "can't read \(args[2])")
}
guard let gain = CGImageSourceCopyAuxiliaryDataInfoAtIndex(src, 0, kCGImageAuxiliaryDataTypeISOGainMap) else {
    fail(2, "\(args[2]) has no ISO gain map")
}
let types = CGImageDestinationCopyTypeIdentifiers() as? [String] ?? []
guard types.contains("public.avif"),
      let dst = CGImageDestinationCreateWithURL(URL(fileURLWithPath: args[3]) as CFURL, "public.avif" as CFString, 1, nil) else {
    fail(3, "this macOS can't write AVIF")
}
CGImageDestinationAddImageFromSource(dst, src, 0, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
CGImageDestinationAddAuxiliaryDataInfo(dst, kCGImageAuxiliaryDataTypeISOGainMap, gain)
guard CGImageDestinationFinalize(dst) else {
    fail(1, "writing \(args[3]) failed")
}
