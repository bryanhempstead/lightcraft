// HDR oracle batch: like tools/lr-compare/oracle/batch.jsx, plus a 4th column: bits (8|16|32).
// Opens each raw through Camera Raw with its own settings (sidecar), saves a ProPhoto TIFF, closes.
(function () {
    var root = "@ROOT@";
    var log = new File(root + "batch.log");
    log.open("a");
    var mf = new File(root + "manifest.txt");
    if (!mf.open("r")) { log.writeln("no manifest"); log.close(); return "no manifest"; }
    var lines = [];
    while (!mf.eof) { var l = mf.readln(); if (l.length > 0) lines.push(l); }
    mf.close();
    var prev = app.displayDialogs;
    app.displayDialogs = DialogModes.NO;
    var ok = 0, bad = 0;
    for (var i = 0; i < lines.length; i++) {
        var parts = lines[i].split("\t");
        var src = new File(parts[0]), dst = new File(parts[1]);
        if (dst.exists) { ok++; continue; }
        try {
            var o = new CameraRAWOpenOptions();
            o.settings = CameraRAWSettingsType.SELECTEDIMAGE;
            o.colorSpace = ColorSpaceType.PROPHOTORGB;
            o.bitsPerChannel = (parts.length > 3 && parts[3] == "32") ? BitsPerChannelType.THIRTYTWO : BitsPerChannelType.SIXTEEN;
            if (parts.length > 2 && parts[2] == "min") o.size = CameraRAWSize.MINIMUM;
            var doc = app.open(src, o);
            log.writeln("open " + parts[0] + " bits " + doc.bitsPerChannel + " " + doc.width + "x" + doc.height + " " + doc.colorProfileName);
            var t = new TiffSaveOptions();
            t.embedColorProfile = true;
            t.imageCompression = TIFFEncoding.NONE;
            doc.saveAs(dst, t, true);
            doc.close(SaveOptions.DONOTSAVECHANGES);
            ok++;
        } catch (e) {
            bad++;
            log.writeln("fail " + parts[0] + ": " + e);
        }
    }
    app.displayDialogs = prev;
    log.writeln("batch done ok " + ok + " bad " + bad);
    log.close();
    return "ok " + ok + " bad " + bad;
})();
