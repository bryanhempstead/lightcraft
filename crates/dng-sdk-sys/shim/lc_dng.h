// C ABI over Adobe's DNG SDK for LightCraft (Bryan's fork). Every function catches every C++
// exception and returns a status: 0 = ok, < 0 = error (see LC_ERR_*), so nothing unwinds into Rust.
#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LC_OK 0
#define LC_ERR_BAD_ARG -1
#define LC_ERR_PARSE -2
#define LC_ERR_SDK -3
#define LC_ERR_SMALL -4
#define LC_ERR_NONE -5

typedef struct lc_profile lc_profile;
typedef struct lc_hsm lc_hsm;

typedef struct lc_profile_info {
    char name[256];
    char calibration_signature[256];
    char unique_model[256];
    uint32_t illuminant1, illuminant2, illuminant3;
    double temperature1, temperature2;
    int32_t has_color_matrix2, has_forward_matrix1, illuminant_model;
    double baseline_exposure_offset;
    uint32_t default_black_render;
    uint32_t hsm_dims[3];
    uint32_t hsm_encoding;
    int32_t has_hsm2;
    uint32_t look_dims[3];
    uint32_t look_encoding;
    uint32_t tone_points;
    uint32_t embed_policy;
    int32_t has_rgb_tables;
    int32_t has_gain_table_map;
} lc_profile_info;

// A camera profile (.dcp: "IIRC" extended profile file) from its bytes.
int lc_profile_open(const uint8_t *data, size_t len, lc_profile **out);
void lc_profile_free(lc_profile *p);
int lc_profile_info_get(const lc_profile *p, lc_profile_info *out);

// dng_color_spec for 3-channel camera data with the profile: the white given as a camera neutral
// (mode 0, in = neutral rgb) or as an xy chromaticity (mode 1, in = {x, y, _}). analog_balance may
// be null (= 1,1,1). Out: white xy, camera white (neutral), camera RGB -> XYZ D50 (PCS) row-major.
int lc_color_spec(const lc_profile *p, const double *analog_balance, int mode, const double in[3],
                  double out_white_xy[2], double out_camera_white[3], double out_camera_to_pcs[9]);

// The profile's ColorMatrix1, ColorMatrix2, ForwardMatrix1, ForwardMatrix2 (3x3 row-major each;
// zeros where absent).
int lc_profile_matrices(const lc_profile *p, double out[36]);

// The profile's hue/sat map interpolated for white xy (DNG dual/triple illuminant), deltas as
// (hue shift deg, sat scale, val scale) triples in val-major, hue, sat-minor order.
// LC_ERR_NONE when the profile has none; LC_ERR_SMALL when cap (floats) is too small (dims set).
int lc_profile_hue_sat_map(const lc_profile *p, double x, double y, uint32_t dims[3], float *buf, size_t cap);
int lc_profile_look_table(const lc_profile *p, uint32_t dims[3], float *buf, size_t cap);

// The profile tone curve (or ACR3 default when the profile has none: returns 1 then) at xs.
int lc_profile_tone_curve(const lc_profile *p, const double *xs, double *ys, size_t n);
int lc_acr3_tone_curve(const double *xs, double *ys, size_t n);

// Camera Raw's Temp / Tint <-> xy (dng_temperature).
int lc_temp_tint_to_xy(double temp, double tint, double out_xy[2]);
int lc_xy_to_temp_tint(double x, double y, double out_tt[2]);

// A Camera Raw XMP look table (crs:Table_<digest> text of a LookTable) decoded with the SDK's
// big-table code: dims, encoding (0 linear, 1 sRGB), amount range {min, max}, deltas as above.
int lc_look_table_decode(const char *text, size_t len, uint32_t dims[3], uint32_t *encoding,
                         double amount_range[2], float *buf, size_t cap);

// A hue/sat map for the reference application: dims, deltas (h*s*v triples), encoding.
int lc_hsm_new(const uint32_t dims[3], const float *deltas, size_t len, uint32_t encoding, lc_hsm **out);
void lc_hsm_free(lc_hsm *h);
// RefBaselineHueSatMap in place on planar linear ProPhoto rows.
int lc_hsm_apply(const lc_hsm *h, float *r, float *g, float *b, size_t n, int overrange);
// RefBaselineRGBTone in place (ProPhoto linear, 0..1) with the profile curve (or ACR3 default).
int lc_rgb_tone(const lc_profile *p, float *r, float *g, float *b, size_t n);

// A Camera Raw XMP RGB table (crs:Table_<digest> of a crs:RGBTable) decoded with the SDK.
// info = {dimensions, divisions, primaries, gamma, gamut}; amount_range = {min, max}.
typedef struct lc_rgbt lc_rgbt;
int lc_rgb_table_new(const char *text, size_t len, uint32_t info[5], double amount_range[2], lc_rgbt **out);
void lc_rgb_table_free(lc_rgbt *t);
// Apply at table amount `amount` in place to planar linear ProPhoto values (the SDK's
// dng_rgb_to_rgb_table_data::Process_32 path).
int lc_rgb_table_apply(const lc_rgbt *t, double amount, float *r, float *g, float *b, size_t n, int overrange);

// Reference render of a DNG with dng_render (optionally with an external profile used instead of
// the file's own), into 16-bit sRGB (final space 0) or ProPhoto linear float (1). w*h*3 samples in
// *out (malloc'd: free with lc_free).
int lc_render_dng(const uint8_t *dng, size_t dng_len, const uint8_t *dcp, size_t dcp_len,
                  uint32_t max_size, int space, uint32_t *w, uint32_t *h, float **out);
void lc_free(void *ptr);

#ifdef __cplusplus
}
#endif
