// LightCraft builds the DNG SDK without libjxl: JPEG XL compressed DNG data (DNG 1.7) is not decoded
// through the SDK (LightCraft's own decoders read the image data). These stand-ins keep the SDK's
// other code linkable and throw "not yet implemented" if a JPEG XL path is ever reached; the shim
// turns that into an error status.

#include "dng_exceptions.h"
#include "dng_jxl.h"

dng_jxl_decoder::~dng_jxl_decoder() {}

void dng_jxl_decoder::Decode(dng_host &, dng_stream &) { ThrowNotYetImplemented("JPEG XL"); }

void dng_jxl_decoder::ProcessExifBox(dng_host &, const std::vector<uint8> &) {}

void dng_jxl_decoder::ProcessXMPBox(dng_host &, const std::vector<uint8> &) {}

void dng_jxl_decoder::ProcessBox(dng_host &, const dng_string &, const std::vector<uint8> &) {}

bool ParseJXL(dng_host &, dng_stream &, dng_info &, bool, bool) { return false; }

void EncodeJXL_Tile(dng_host &, dng_stream &, const dng_pixel_buffer &, const dng_jxl_color_space_info &, const dng_jxl_encode_settings &) {
    ThrowNotYetImplemented("JPEG XL");
}

void EncodeJXL_Tile(dng_host &, dng_stream &, const dng_image &, const dng_jxl_color_space_info &, const dng_jxl_encode_settings &) {
    ThrowNotYetImplemented("JPEG XL");
}

void EncodeJXL_Container(dng_host &, dng_stream &, const dng_image &, const dng_jxl_encode_settings &, const dng_jxl_color_space_info &,
                         const dng_metadata *, const bool, const bool, const bool, const dng_bmff_box_list *) {
    ThrowNotYetImplemented("JPEG XL");
}

void EncodeJXL_Container(dng_host &, dng_stream &, const dng_pixel_buffer &, const dng_jxl_encode_settings &, const dng_jxl_color_space_info &,
                         const dng_metadata *, const bool, const bool, const bool, const dng_bmff_box_list *) {
    ThrowNotYetImplemented("JPEG XL");
}

void PreviewColorSpaceToJXLEncoding(const PreviewColorSpaceEnum, const uint32, dng_jxl_color_space_info &) {
    ThrowNotYetImplemented("JPEG XL");
}

bool SupportsJXL(const dng_image &) { return false; }
