// LightCraft's C ABI over LibRaw: open + unpack only (no demosaic, no colour processing), every
// exception caught. See lc_libraw.h.
#include "lc_libraw.h"

#include <cstring>
#include <new>

#include "libraw/libraw.h"

namespace {

void count_data_error(void *data, const char *, const INT64) {
  if (data) ++*static_cast<uint32_t *>(data);
}

void copy_str(char *dst, size_t cap, const char *src) {
  if (cap == 0) return;
  if (!src) {
    dst[0] = 0;
    return;
  }
  size_t n = strnlen(src, cap - 1);
  memcpy(dst, src, n);
  dst[n] = 0;
}

template <size_t N> void copy_arr(char (&dst)[N], const char *src) { copy_str(dst, N, src); }

void fill(LibRaw *lr, lc_libraw_info *o) {
  const libraw_data_t &d = lr->imgdata;
  memset(o, 0, sizeof(*o));
  o->raw_width = d.sizes.raw_width;
  o->raw_height = d.sizes.raw_height;
  o->width = d.sizes.width;
  o->height = d.sizes.height;
  o->top_margin = d.sizes.top_margin;
  o->left_margin = d.sizes.left_margin;
  o->crop_left = d.sizes.raw_inset_crops[0].cleft;
  o->crop_top = d.sizes.raw_inset_crops[0].ctop;
  o->crop_width = d.sizes.raw_inset_crops[0].cwidth;
  o->crop_height = d.sizes.raw_inset_crops[0].cheight;
  o->flip = d.sizes.flip;
  o->colors = d.idata.colors;
  o->filters = d.idata.filters;
  memcpy(o->xtrans, d.idata.xtrans, sizeof(o->xtrans));
  copy_arr(o->cdesc, d.idata.cdesc);
  o->dng_version = d.idata.dng_version;
  o->is_foveon = d.idata.is_foveon;
  const libraw_rawdata_t &r = d.rawdata;
  if (r.raw_image) o->channels = 1;
  else if (r.color3_image) o->channels = 3;
  else if (r.color4_image) o->channels = 4;
  else if (r.float_image) { o->channels = 1; o->is_float = 1; }
  else if (r.float3_image) { o->channels = 3; o->is_float = 1; }
  else if (r.float4_image) { o->channels = 4; o->is_float = 1; }
  o->black = d.color.black;
  o->maximum = d.color.maximum;
  o->data_maximum = d.color.data_maximum;
  o->raw_bps = d.color.raw_bps;
  for (int c = 0; c < 4; c++) o->linear_max[c] = d.color.linear_max[c] > 0 ? (uint32_t)d.color.linear_max[c] : 0;
  memcpy(o->cblack, d.color.cblack, sizeof(o->cblack));
  memcpy(o->cam_mul, d.color.cam_mul, sizeof(o->cam_mul));
  memcpy(o->pre_mul, d.color.pre_mul, sizeof(o->pre_mul));
  memcpy(o->cam_xyz, d.color.cam_xyz, sizeof(o->cam_xyz));
  memcpy(o->rgb_cam, d.color.rgb_cam, sizeof(o->rgb_cam));
  for (int k = 0; k < 2; k++) {
    memcpy(o->dng_colormatrix[k], d.color.dng_color[k].colormatrix, sizeof(o->dng_colormatrix[k]));
    o->dng_illuminant[k] = d.color.dng_color[k].illuminant ? d.color.dng_color[k].illuminant : 0xffff;
  }
  o->iso_speed = d.other.iso_speed;
  o->shutter = d.other.shutter;
  o->aperture = d.other.aperture;
  o->focal_len = d.other.focal_len;
  o->timestamp = (int64_t)d.other.timestamp;
  copy_arr(o->make, d.idata.make);
  copy_arr(o->model, d.idata.model);
  copy_arr(o->normalized_make, d.idata.normalized_make);
  copy_arr(o->normalized_model, d.idata.normalized_model);
  copy_arr(o->software, d.idata.software);
  copy_arr(o->lens, d.lens.Lens[0] ? d.lens.Lens : d.lens.makernotes.Lens);
  copy_arr(o->lens_make, d.lens.LensMake);
  copy_arr(o->decoder, lr->unpack_function_name());
}

}  // namespace

extern "C" void *lc_libraw_open(const uint8_t *data, size_t len, int unpack, uint32_t max_mb, lc_libraw_info *out, int *err) {
  int dummy = 0;
  if (!err) err = &dummy;
  *err = LC_LR_OK;
  if (!data || len == 0 || !out) {
    *err = LC_LR_BAD_ARGUMENT;
    return nullptr;
  }
  LibRaw *lr = nullptr;
  try {
    lr = new (std::nothrow) LibRaw(LIBRAW_OPTIONS_NO_DATAERR_CALLBACK);
    if (!lr) {
      *err = LC_LR_NO_MEMORY;
      return nullptr;
    }
    uint32_t data_errors = 0;
    lr->set_dataerror_handler(count_data_error, &data_errors);
    lr->imgdata.rawparams.use_rawspeed = 0;
    lr->imgdata.rawparams.use_dngsdk = 0;
    if (max_mb > 0) lr->imgdata.rawparams.max_raw_memory_mb = max_mb;
    int rc = lr->open_buffer(data, len);
    if (rc == LIBRAW_SUCCESS && unpack) rc = lr->unpack();
    if (rc != LIBRAW_SUCCESS) {
      *err = rc;
      lr->set_dataerror_handler(nullptr, nullptr);
      delete lr;
      return nullptr;
    }
    fill(lr, out);
    out->data_errors = data_errors;
    // the counter lives on this stack frame: no callback after this call returns
    lr->set_dataerror_handler(nullptr, nullptr);
    return lr;
  } catch (const std::bad_alloc &) {
    *err = LC_LR_NO_MEMORY;
  } catch (...) {
    *err = LC_LR_EXCEPTION;
  }
  delete lr;
  return nullptr;
}

extern "C" int lc_libraw_copy_samples(void *handle, uint16_t *dst, size_t n) {
  if (!handle || !dst) return LC_LR_BAD_ARGUMENT;
  try {
    LibRaw *lr = static_cast<LibRaw *>(handle);
    const libraw_rawdata_t &r = lr->imgdata.rawdata;
    const size_t w = lr->imgdata.sizes.raw_width, h = lr->imgdata.sizes.raw_height;
    const uint16_t *src = nullptr;
    size_t ch = 0;
    if (r.raw_image) { src = r.raw_image; ch = 1; }
    else if (r.color3_image) { src = &r.color3_image[0][0]; ch = 3; }
    else if (r.color4_image) { src = &r.color4_image[0][0]; ch = 4; }
    if (!src) return LC_LR_NO_SAMPLES;
    const size_t row = w * ch;
    if (w == 0 || h == 0 || n / h / ch != w || n != row * h) return LC_LR_BAD_ARGUMENT;
    // raw_pitch: bytes per stored row
    size_t stride = lr->imgdata.sizes.raw_pitch / sizeof(uint16_t);
    if (stride < row) stride = row;
    for (size_t y = 0; y < h; y++) memcpy(dst + y * row, src + y * stride, row * sizeof(uint16_t));
    return LC_LR_OK;
  } catch (...) {
    return LC_LR_EXCEPTION;
  }
}

extern "C" void lc_libraw_close(void *handle) {
  try {
    delete static_cast<LibRaw *>(handle);
  } catch (...) {
  }
}

extern "C" const char *lc_libraw_version(void) { return LibRaw::version(); }

extern "C" int lc_libraw_camera_count(void) { return LibRaw::cameraCount(); }

extern "C" const char *lc_libraw_strerror(int code) { return LibRaw::strerror(code); }
