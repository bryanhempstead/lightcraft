// LightCraft's AVFoundation video helper (our own source; built once with swiftc by video.rs).
//
//   lc-video probe <path>                         → one JSON line: width, height (upright), duration, fps, codec, audio, created
//   lc-video frame <path> <secs> <maxEdge> <out>  → writes the upright frame at <secs> as a PNG, no larger than maxEdge
//   lc-video stream <path> <from> <to> <w> <h>    → raw RGBA frames of exactly w×h (upright, scaled) on stdout, from..to
//
// Errors go to stderr with a non-zero exit. Used when ffmpeg isn't installed.

import AVFoundation
import CoreImage
import Foundation
import ImageIO
import UniformTypeIdentifiers

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(1)
}

func asset(_ path: String) -> AVURLAsset {
    AVURLAsset(url: URL(fileURLWithPath: path), options: [AVURLAssetPreferPreciseDurationAndTimingKey: true])
}

func uprightSize(_ track: AVAssetTrack) -> CGSize {
    let r = CGRect(origin: .zero, size: track.naturalSize).applying(track.preferredTransform)
    return CGSize(width: abs(r.width), height: abs(r.height))
}

let args = CommandLine.arguments
if args.count < 3 { fail("usage: lc-video probe|frame|stream <path> …") }
let path = args[2]
let a = asset(path)

switch args[1] {
case "probe":
    guard let v = a.tracks(withMediaType: .video).first else { fail("no video track") }
    let size = uprightSize(v)
    let dur = CMTimeGetSeconds(a.duration)
    var created = ""
    if let d = a.creationDate?.dateValue {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withFullDate, .withTime, .withColonSeparatorInTime, .withDashSeparatorInDate]
        created = f.string(from: d)
    }
    let codec: String = {
        guard let desc = v.formatDescriptions.first else { return "" }
        let fourcc = CMFormatDescriptionGetMediaSubType(desc as! CMFormatDescription)
        let chars = [24, 16, 8, 0].map { Character(UnicodeScalar(UInt8((fourcc >> $0) & 0xff))) }
        return String(chars).trimmingCharacters(in: .whitespaces)
    }()
    let obj: [String: Any] = [
        "width": Int(size.width.rounded()), "height": Int(size.height.rounded()),
        "duration": dur.isFinite ? dur : 0, "fps": Double(v.nominalFrameRate),
        "codec": codec, "audio": !a.tracks(withMediaType: .audio).isEmpty, "created": created,
    ]
    let data = try! JSONSerialization.data(withJSONObject: obj)
    FileHandle.standardOutput.write(data)
case "frame":
    if args.count < 6 { fail("usage: lc-video frame <path> <secs> <maxEdge> <out.png>") }
    let secs = Double(args[3]) ?? 0
    let edge = Double(args[4]) ?? 0
    let g = AVAssetImageGenerator(asset: a)
    g.appliesPreferredTrackTransform = true
    g.requestedTimeToleranceBefore = CMTime(value: 1, timescale: 30)
    g.requestedTimeToleranceAfter = CMTime(value: 1, timescale: 30)
    if edge > 0 { g.maximumSize = CGSize(width: edge, height: edge) }
    let img: CGImage
    do { img = try g.copyCGImage(at: CMTime(seconds: max(0, secs), preferredTimescale: 600), actualTime: nil) } catch { fail("no frame: \(error.localizedDescription)") }
    let url = URL(fileURLWithPath: args[5])
    guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else { fail("can't write \(args[5])") }
    CGImageDestinationAddImage(dest, img, nil)
    if !CGImageDestinationFinalize(dest) { fail("can't write \(args[5])") }
case "stream":
    if args.count < 7 { fail("usage: lc-video stream <path> <from> <to> <w> <h>") }
    let from = Double(args[3]) ?? 0
    let to = Double(args[4]) ?? 0
    let w = Int(args[5]) ?? 0
    let h = Int(args[6]) ?? 0
    if w <= 0 || h <= 0 || w > 8192 || h > 8192 { fail("bad size") }
    guard let v = a.tracks(withMediaType: .video).first else { fail("no video track") }
    let reader: AVAssetReader
    do { reader = try AVAssetReader(asset: a) } catch { fail("can't read: \(error.localizedDescription)") }
    let start = CMTime(seconds: max(0, from), preferredTimescale: 600)
    let end = to > from ? CMTime(seconds: to, preferredTimescale: 600) : a.duration
    reader.timeRange = CMTimeRange(start: start, end: end)
    let out = AVAssetReaderTrackOutput(track: v, outputSettings: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA])
    out.alwaysCopiesSampleData = false
    reader.add(out)
    if !reader.startReading() { fail("can't read: \(reader.error?.localizedDescription ?? "")") }
    let ctx = CIContext(options: [.workingColorSpace: NSNull(), .outputColorSpace: NSNull()])
    let t = v.preferredTransform
    let natural = v.naturalSize
    var buf = [UInt8](repeating: 0, count: w * h * 4)
    let stdout = FileHandle.standardOutput
    while let sample = out.copyNextSampleBuffer() {
        guard let px = CMSampleBufferGetImageBuffer(sample) else { continue }
        // the track's transform (rotation) in CoreImage's bottom-left frame, then scaled to w×h
        var img = CIImage(cvPixelBuffer: px)
        let flip = CGAffineTransform(a: t.a, b: -t.b, c: -t.c, d: t.d, tx: 0, ty: 0)
        img = img.transformed(by: flip)
        img = img.transformed(by: CGAffineTransform(translationX: -img.extent.minX, y: -img.extent.minY))
        let up = img.extent.size
        _ = natural
        img = img.transformed(by: CGAffineTransform(scaleX: CGFloat(w) / max(up.width, 1), y: CGFloat(h) / max(up.height, 1)))
        buf.withUnsafeMutableBytes { p in
            ctx.render(img, toBitmap: p.baseAddress!, rowBytes: w * 4, bounds: CGRect(x: 0, y: 0, width: w, height: h), format: .RGBA8, colorSpace: nil)
        }
        do { try stdout.write(contentsOf: Data(buf)) } catch { exit(0) }  // the reader went away: stop
    }
default:
    fail("unknown command \(args[1])")
}
