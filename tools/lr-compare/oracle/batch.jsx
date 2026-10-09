// Camera Raw oracle batch: opens each DNG listed in manifest.txt (one path per line, embedded crs
// settings) through Camera Raw with the image's own settings, saves a 16-bit ProPhoto TIFF next to
// the output path, closes without saving. Restores the dialog mode it changed.
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
        try {
            var o = new CameraRAWOpenOptions();
            o.settings = CameraRAWSettingsType.SELECTEDIMAGE;
            o.colorSpace = ColorSpaceType.PROPHOTORGB;
            o.bitsPerChannel = BitsPerChannelType.SIXTEEN;
            if (parts.length > 2 && parts[2] == "min") o.size = CameraRAWSize.MINIMUM;
            var doc = app.open(src, o);
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
