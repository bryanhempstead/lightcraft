/* LightCraft's C ABI over LibRaw (crates/libraw-sys). Every entry point catches every C++
 * exception and reports it as an error code; nothing here throws across the ABI. */
#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Error codes (besides LibRaw's own negative LIBRAW_* codes). */
#define LC_LR_OK 0
#define LC_LR_BAD_ARGUMENT 1
#define LC_LR_EXCEPTION 2
#define LC_LR_NO_MEMORY 3
#define LC_LR_NO_SAMPLES 4

#define LC_LR_CBLACK 4104

/* Everything LightCraft reads from a LibRaw decode (no demosaic, no colour processing). */
typedef struct lc_libraw_info {
  /* geometry (sizes): the full sensor frame and the visible area inside it */
  uint32_t raw_width, raw_height, width, height, top_margin, left_margin;
  /* raw_inset_crops[0]: the camera's default crop in the full frame (cleft 0xffff = none) */
  uint32_t crop_left, crop_top, crop_width, crop_height;
  /* LibRaw's orientation code (0 none, 3 180°, 5 90° CCW, 6 90° CW, others flips) */
  int32_t flip;
  /* colour filter array (iparams): filters 0 = none, 9 = X-Trans, 1 = Leaf 16×16, else Bayer bits;
   * xtrans is relative to the visible area; cdesc names the colour indices ("RGBG") */
  int32_t colors;
  uint32_t filters;
  char xtrans[6][6];
  char cdesc[8];
  uint32_t dng_version, is_foveon;
  /* samples per pixel of the unpacked data (1 = CFA / mono, 3, 4), 0 before unpacking;
   * is_float = the data is floating point (not copied: LightCraft reads float DNGs itself) */
  uint32_t channels, is_float;
  /* levels: black + cblack[c] (c = colour index) + cblack[6 + (row % cblack[4]) * cblack[5]
   * + col % cblack[5]] (visible-area coordinates); maximum = white */
  uint32_t black, maximum, data_maximum, raw_bps;
  /* the highest linear value per colour from the maker notes (Canon SpecularWhiteLevel, …), 0 = none */
  uint32_t linear_max[4];
  uint32_t cblack[LC_LR_CBLACK];
  /* colour: as-shot multipliers (R G B G2), daylight multipliers, XYZ (D65) → camera matrix
   * (Adobe's ColorMatrix where LibRaw knows the camera), camera → sRGB */
  float cam_mul[4], pre_mul[4], cam_xyz[4][3], rgb_cam[3][4];
  /* a DNG's own ColorMatrix1/2 and their illuminants (0 = not present) */
  float dng_colormatrix[2][4][3];
  uint32_t dng_illuminant[2];
  /* shooting */
  float iso_speed, shutter, aperture, focal_len;
  int64_t timestamp;
  char make[64], model[64], normalized_make[64], normalized_model[64], software[64];
  char lens[128], lens_make[128];
  /* how often LibRaw's decoders hit corrupt data (its "data error" callback) */
  uint32_t data_errors;
  /* LibRaw's decoder for this file (e.g. "crxLoadRaw()") */
  char decoder[64];
} lc_libraw_info;

/* Open `data` (it must stay valid until lc_libraw_close), unpack the sensor data when `unpack`,
 * fill `out`. Returns a handle, or NULL with `*err` set. */
void *lc_libraw_open(const uint8_t *data, size_t len, int unpack, uint32_t max_mb, lc_libraw_info *out, int *err);
/* Copy the unpacked 16-bit samples, raw_width × raw_height × channels, row-major. `n` must be
 * exactly that count. */
int lc_libraw_copy_samples(void *handle, uint16_t *dst, size_t n);
void lc_libraw_close(void *handle);

const char *lc_libraw_version(void);
int lc_libraw_camera_count(void);
/* LibRaw's text for one of its error codes (static storage). */
const char *lc_libraw_strerror(int code);

#ifdef __cplusplus
}
#endif
