// LightCraft face finder (macOS): Apple Vision face rectangles for each image path given.
// Prints one JSON array per call: for each image, an array of {"x","y","w","h","c"} — normalized,
// top-left origin, y down; c = Vision's confidence. An unreadable image gives [].
// Built once by crates/engine/src/faces.rs (swiftc) into LightCraft's support folder.
import CoreGraphics
import Foundation
import ImageIO
import Vision

var all: [[[String: Double]]] = []
for path in CommandLine.arguments.dropFirst() {
    var faces: [[String: Double]] = []
    if let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
       let img = CGImageSourceCreateImageAtIndex(src, 0, nil) {
        let req = VNDetectFaceRectanglesRequest()
        let handler = VNImageRequestHandler(cgImage: img, options: [:])
        if (try? handler.perform([req])) != nil {
            for f in req.results ?? [] {
                let b = f.boundingBox
                faces.append(["x": Double(b.minX), "y": Double(1 - b.maxY), "w": Double(b.width), "h": Double(b.height), "c": Double(f.confidence)])
            }
        }
    }
    all.append(faces)
}
if let data = try? JSONSerialization.data(withJSONObject: all), let text = String(data: data, encoding: .utf8) {
    print(text)
} else {
    print("[]")
}
