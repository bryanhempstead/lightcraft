// LightCraft's "move to Trash" helper (original work, MIT OR Apache-2.0): moves each path given on
// the command line to the Trash with FileManager.trashItem — the Finder's own Trash, so "Put Back"
// works. Prints one JSON line per path: {"path": …, "trashed": …} or {"path": …, "error": …}.
import Foundation

func esc(_ s: String) -> String {
    var o = ""
    for c in s.unicodeScalars {
        switch c {
        case "\"": o += "\\\""
        case "\\": o += "\\\\"
        case "\n": o += "\\n"
        case "\r": o += "\\r"
        case "\t": o += "\\t"
        default:
            if c.value < 0x20 { o += String(format: "\\u%04x", c.value) } else { o.unicodeScalars.append(c) }
        }
    }
    return o
}

for path in CommandLine.arguments.dropFirst() {
    let url = URL(fileURLWithPath: path)
    do {
        var out: NSURL? = nil
        try FileManager.default.trashItem(at: url, resultingItemURL: &out)
        print("{\"path\":\"\(esc(path))\",\"trashed\":\"\(esc(out?.path ?? ""))\"}")
    } catch {
        print("{\"path\":\"\(esc(path))\",\"error\":\"\(esc(error.localizedDescription))\"}")
    }
}
