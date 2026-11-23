// hdrdecode IN OUT.f32 [sdr]: decode an image with macOS ImageIO as HDR (gain map applied, as Preview /
// Photos do) or as SDR, into linear extended Rec.2020 floats (SDR white = 1): [w, h] u32 then w·h·3 f32.
import Foundation
import ImageIO
import CoreGraphics

let a = CommandLine.arguments
guard a.count >= 3, let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[1]) as CFURL, nil) else { print("usage"); exit(2) }
var opts: [CFString: Any] = [:]
if a.count < 4 { opts[kCGImageSourceDecodeRequest] = kCGImageSourceDecodeToHDR }
guard let img = CGImageSourceCreateImageAtIndex(src, 0, opts as CFDictionary) else { print("no image"); exit(3) }
let (w, h) = (img.width, img.height)
guard let cs = CGColorSpace(name: CGColorSpace.extendedLinearITUR_2020),
      let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 32, bytesPerRow: w * 16, space: cs,
                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.floatComponents.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
else { print("no context"); exit(4) }
ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
guard let p = ctx.data else { exit(5) }
let f = p.bindMemory(to: Float.self, capacity: w * h * 4)
var out = Data()
var dims: [UInt32] = [UInt32(w), UInt32(h)]
out.append(Data(bytes: &dims, count: 8))
var rgb = [Float](repeating: 0, count: w * h * 3)
for i in 0..<(w * h) { for c in 0..<3 { rgb[i * 3 + c] = f[i * 4 + c] } }
rgb.withUnsafeBytes { out.append(contentsOf: $0) }
try! out.write(to: URL(fileURLWithPath: a[2]))
var mx: Float = 0
for v in rgb { mx = max(mx, v) }
print("\(w)x\(h) max \(mx) colorspace \(img.colorSpace?.name as String? ?? "?")")
