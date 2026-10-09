// C ABI over Adobe's DNG SDK for LightCraft. See lc_dng.h. Every entry point catches every
// exception (dng_exception, std::bad_alloc, anything) and reports a status instead.

#include "lc_dng.h"

#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <string>
#include <vector>

#include "dng_1d_table.h"
#include "dng_auto_ptr.h"
#include "dng_big_table.h"
#include "dng_camera_profile.h"
#include "dng_color_space.h"
#include "dng_color_spec.h"
#include "dng_exceptions.h"
#include "dng_host.h"
#include "dng_hue_sat_map.h"
#include "dng_image.h"
#include "dng_info.h"
#include "dng_memory.h"
#include "dng_negative.h"
#include "dng_pixel_buffer.h"
#include "dng_reference.h"
#include "dng_render.h"
#include "dng_spline.h"
#include "dng_stream.h"
#include "dng_string.h"
#include "dng_temperature.h"
#include "dng_tone_curve.h"
#include "dng_xy_coord.h"

struct lc_profile {
    dng_camera_profile profile;
};

struct lc_rgbt {
    dng_rgb_table table;
};

struct lc_hsm {
    dng_hue_sat_map map;
    AutoPtr<dng_1d_table> encode;
    AutoPtr<dng_1d_table> decode;
};

namespace {

template <typename F> int guarded(F &&f) {
    try {
        return f();
    } catch (const dng_exception &) {
        return LC_ERR_SDK;
    } catch (const std::bad_alloc &) {
        return LC_ERR_SDK;
    } catch (...) {
        return LC_ERR_SDK;
    }
}

void copy_string(char *dst, size_t cap, const dng_string &s) {
    if (cap == 0) return;
    const char *src = s.Get();
    size_t n = src ? strnlen(src, cap - 1) : 0;
    if (n) memcpy(dst, src, n);
    dst[n] = 0;
}

void dims_of(const dng_hue_sat_map &m, uint32_t dims[3]) {
    uint32 h = 0, s = 0, v = 0;
    m.GetDivisions(h, s, v);
    dims[0] = h;
    dims[1] = s;
    dims[2] = v;
}

int copy_map(const dng_hue_sat_map &m, uint32_t dims[3], float *buf, size_t cap) {
    if (!m.IsValid()) return LC_ERR_NONE;
    dims_of(m, dims);
    size_t count = m.DeltasCount();
    if (!buf || cap < count * 3) return LC_ERR_SMALL;
    const dng_hue_sat_map::HSBModify *d = m.GetConstDeltas();
    for (size_t i = 0; i < count; i++) {
        buf[i * 3] = d[i].fHueShift;
        buf[i * 3 + 1] = d[i].fSatScale;
        buf[i * 3 + 2] = d[i].fValScale;
    }
    return LC_OK;
}

// A 3-channel negative for colour-spec math (analog balance optional).
dng_negative *make_negative(dng_host &host, const double *analog_balance) {
    AutoPtr<dng_negative> negative(host.Make_dng_negative());
    negative->SetColorChannels(3);
    if (analog_balance) {
        dng_vector b(3);
        for (uint32 i = 0; i < 3; i++) b[i] = analog_balance[i];
        negative->SetAnalogBalance(b);
    }
    return negative.Release();
}

// dng_rgb_table keeps its string decoder protected too.
class open_rgb_table : public dng_rgb_table {
public:
    bool Decode(dng_host &host, const dng_string &s) { return DecodeFromString(host, s); }
};

// dng_look_table keeps its string decoder protected.
class open_look_table : public dng_look_table {
public:
    bool Decode(dng_host &host, const dng_string &s) { return DecodeFromString(host, s); }
};

const dng_1d_function &curve_of(const dng_camera_profile *p, dng_spline_solver &solver, int &own) {
    own = 0;
    if (p && p->ToneCurve().IsValid()) {
        p->ToneCurve().Solve(solver);
        own = 1;
        return solver;
    }
    return dng_tone_curve_acr3_default::Get();
}

}  // namespace

extern "C" {

int lc_profile_open(const uint8_t *data, size_t len, lc_profile **out) {
    if (!data || !out || len < 8 || len > 0x7fffffff) return LC_ERR_BAD_ARG;
    *out = nullptr;
    return guarded([&] {
        std::unique_ptr<lc_profile> p(new lc_profile());
        dng_stream stream(data, (uint32)len);
        if (!p->profile.ParseExtended(stream)) return LC_ERR_PARSE;
        if (!p->profile.IsValid(3)) return LC_ERR_PARSE;
        *out = p.release();
        return LC_OK;
    });
}

void lc_profile_free(lc_profile *p) { delete p; }

int lc_profile_info_get(const lc_profile *p, lc_profile_info *o) {
    if (!p || !o) return LC_ERR_BAD_ARG;
    return guarded([&] {
        memset(o, 0, sizeof(*o));
        const dng_camera_profile &c = p->profile;
        copy_string(o->name, sizeof(o->name), c.Name());
        copy_string(o->calibration_signature, sizeof(o->calibration_signature), c.ProfileCalibrationSignature());
        copy_string(o->unique_model, sizeof(o->unique_model), c.UniqueCameraModelRestriction());
        o->illuminant1 = c.CalibrationIlluminant1();
        o->illuminant2 = c.CalibrationIlluminant2();
        o->illuminant3 = c.CalibrationIlluminant3();
        o->temperature1 = c.CalibrationTemperature1();
        o->temperature2 = c.CalibrationTemperature2();
        o->has_color_matrix2 = c.HasColorMatrix2() ? 1 : 0;
        o->has_forward_matrix1 = c.ForwardMatrix1().NotEmpty() ? 1 : 0;
        o->illuminant_model = (int32_t)c.IlluminantModel();
        o->baseline_exposure_offset = c.BaselineExposureOffset().As_real64();
        o->default_black_render = c.DefaultBlackRender();
        if (c.HasHueSatDeltas()) {
            dims_of(c.HueSatDeltas1(), o->hsm_dims);
            o->has_hsm2 = c.HueSatDeltas2().IsValid() ? 1 : 0;
        }
        o->hsm_encoding = c.HueSatMapEncoding();
        if (c.HasLookTable()) dims_of(c.LookTable(), o->look_dims);
        o->look_encoding = c.LookTableEncoding();
        o->tone_points = c.ToneCurve().IsValid() ? (uint32_t)c.ToneCurve().fCoord.size() : 0;
        o->embed_policy = c.EmbedPolicy();
        o->has_rgb_tables = c.HasMaskedRGBTables() ? 1 : 0;
        o->has_gain_table_map = c.HasProfileGainTableMap() ? 1 : 0;
        return LC_OK;
    });
}

int lc_color_spec(const lc_profile *p, const double *analog_balance, int mode, const double in[3],
                  double out_white_xy[2], double out_camera_white[3], double out_camera_to_pcs[9]) {
    if (!p || !in || !out_white_xy || !out_camera_white || !out_camera_to_pcs) return LC_ERR_BAD_ARG;
    return guarded([&] {
        dng_host host;
        AutoPtr<dng_negative> negative(make_negative(host, analog_balance));
        dng_color_spec spec(*negative, &p->profile);
        dng_xy_coord white;
        if (mode == 0) {
            dng_vector n(3);
            for (uint32 i = 0; i < 3; i++) {
                if (!(in[i] > 0.0)) return LC_ERR_BAD_ARG;
                n[i] = in[i];
            }
            white = spec.NeutralToXY(n);
        } else {
            if (!(in[0] > 0.0 && in[1] > 0.0)) return LC_ERR_BAD_ARG;
            white = dng_xy_coord(in[0], in[1]);
        }
        spec.SetWhiteXY(white);
        out_white_xy[0] = spec.WhiteXY().x;
        out_white_xy[1] = spec.WhiteXY().y;
        const dng_vector &cw = spec.CameraWhite();
        const dng_matrix &m = spec.CameraToPCS();
        if (cw.Count() != 3 || m.Rows() != 3 || m.Cols() != 3) return LC_ERR_SDK;
        for (uint32 i = 0; i < 3; i++) {
            out_camera_white[i] = cw[i];
            for (uint32 j = 0; j < 3; j++) out_camera_to_pcs[i * 3 + j] = m[i][j];
        }
        return LC_OK;
    });
}

int lc_profile_matrices(const lc_profile *p, double out[36]) {
    if (!p || !out) return LC_ERR_BAD_ARG;
    return guarded([&] {
        const dng_matrix *ms[4] = {&p->profile.ColorMatrix1(), &p->profile.ColorMatrix2(), &p->profile.ForwardMatrix1(), &p->profile.ForwardMatrix2()};
        for (int k = 0; k < 4; k++)
            for (uint32 i = 0; i < 3; i++)
                for (uint32 j = 0; j < 3; j++)
                    out[k * 9 + i * 3 + j] = (ms[k]->Rows() == 3 && ms[k]->Cols() == 3) ? (*ms[k])[i][j] : 0.0;
        return LC_OK;
    });
}

int lc_profile_hue_sat_map(const lc_profile *p, double x, double y, uint32_t dims[3], float *buf, size_t cap) {
    if (!p || !dims) return LC_ERR_BAD_ARG;
    return guarded([&] {
        AutoPtr<dng_hue_sat_map> m(p->profile.HueSatMapForWhite(dng_xy_coord(x, y)));
        if (!m.Get()) return LC_ERR_NONE;
        return copy_map(*m, dims, buf, cap);
    });
}

int lc_profile_look_table(const lc_profile *p, uint32_t dims[3], float *buf, size_t cap) {
    if (!p || !dims) return LC_ERR_BAD_ARG;
    return guarded([&] {
        if (!p->profile.HasLookTable()) return LC_ERR_NONE;
        return copy_map(p->profile.LookTable(), dims, buf, cap);
    });
}

int lc_profile_tone_curve(const lc_profile *p, const double *xs, double *ys, size_t n) {
    if (!xs || !ys) return LC_ERR_BAD_ARG;
    return guarded([&] {
        dng_spline_solver solver;
        int own = 0;
        const dng_1d_function &f = curve_of(p ? &p->profile : nullptr, solver, own);
        for (size_t i = 0; i < n; i++) ys[i] = f.Evaluate(Pin_real64(0.0, xs[i], 1.0));
        return own;
    });
}

int lc_acr3_tone_curve(const double *xs, double *ys, size_t n) { return lc_profile_tone_curve(nullptr, xs, ys, n) < 0 ? LC_ERR_SDK : LC_OK; }

int lc_temp_tint_to_xy(double temp, double tint, double out_xy[2]) {
    if (!out_xy || !(temp > 0.0)) return LC_ERR_BAD_ARG;
    return guarded([&] {
        dng_temperature t(temp, tint);
        dng_xy_coord xy = t.Get_xy_coord();
        out_xy[0] = xy.x;
        out_xy[1] = xy.y;
        return LC_OK;
    });
}

int lc_xy_to_temp_tint(double x, double y, double out_tt[2]) {
    if (!out_tt || !(x > 0.0 && y > 0.0)) return LC_ERR_BAD_ARG;
    return guarded([&] {
        dng_temperature t(dng_xy_coord(x, y));
        out_tt[0] = t.Temperature();
        out_tt[1] = t.Tint();
        return LC_OK;
    });
}

int lc_look_table_decode(const char *text, size_t len, uint32_t dims[3], uint32_t *encoding,
                         double amount_range[2], float *buf, size_t cap) {
    if (!text || !dims || !encoding || !amount_range || len == 0 || len > (64u << 20)) return LC_ERR_BAD_ARG;
    return guarded([&] {
        dng_host host;
        dng_string s;
        s.Set_UTF8_or_System(std::string(text, len).c_str());
        open_look_table t;
        if (!t.Decode(host, s) || !t.IsValid()) return LC_ERR_PARSE;
        *encoding = t.Encoding();
        amount_range[0] = t.MinAmount();
        amount_range[1] = t.MaxAmount();
        return copy_map(t.Map(), dims, buf, cap);
    });
}

int lc_hsm_new(const uint32_t dims[3], const float *deltas, size_t len, uint32_t encoding, lc_hsm **out) {
    if (!dims || !deltas || !out) return LC_ERR_BAD_ARG;
    *out = nullptr;
    uint64_t h = dims[0], s = dims[1], v = dims[2] == 0 ? 1 : dims[2];
    if (h < 1 || h > 360 || s < 2 || s > 256 || v > 256 || h * s * v * 3 != len) return LC_ERR_BAD_ARG;
    return guarded([&] {
        std::unique_ptr<lc_hsm> m(new lc_hsm());
        m->map.SetDivisions((uint32)h, (uint32)s, (uint32)v);
        dng_hue_sat_map::HSBModify *d = m->map.GetDeltas();
        if (!d) return LC_ERR_SDK;
        for (size_t i = 0; i < len / 3; i++) {
            d[i].fHueShift = deltas[i * 3];
            d[i].fSatScale = deltas[i * 3 + 1];
            d[i].fValScale = deltas[i * 3 + 2];
        }
        m->map.AssignNewUniqueRuntimeFingerprint();
        if (encoding != 0) BuildHueSatMapEncodingTable(gDefaultDNGMemoryAllocator, encoding, m->encode, m->decode, false);
        *out = m.release();
        return LC_OK;
    });
}

void lc_hsm_free(lc_hsm *h) { delete h; }

int lc_hsm_apply(const lc_hsm *h, float *r, float *g, float *b, size_t n, int overrange) {
    if (!h || !r || !g || !b || n > 0xffffffffu) return LC_ERR_BAD_ARG;
    return guarded([&] {
        RefBaselineHueSatMap(r, g, b, r, g, b, (uint32)n, h->map, h->encode.Get(), h->decode.Get(), overrange != 0);
        return LC_OK;
    });
}

int lc_rgb_tone(const lc_profile *p, float *r, float *g, float *b, size_t n) {
    if (!r || !g || !b || n > 0xffffffffu) return LC_ERR_BAD_ARG;
    return guarded([&] {
        dng_spline_solver solver;
        int own = 0;
        const dng_1d_function &f = curve_of(p ? &p->profile : nullptr, solver, own);
        dng_1d_table table;
        table.Initialize(gDefaultDNGMemoryAllocator, f);
        RefBaselineRGBTone(r, g, b, r, g, b, (uint32)n, table);
        return LC_OK;
    });
}

int lc_rgb_table_new(const char *text, size_t len, uint32_t info[5], double amount_range[2], lc_rgbt **out) {
    if (!text || !info || !amount_range || !out || len == 0 || len > (64u << 20)) return LC_ERR_BAD_ARG;
    *out = nullptr;
    return guarded([&] {
        dng_host host;
        dng_string s;
        s.Set_UTF8_or_System(std::string(text, len).c_str());
        open_rgb_table t;
        if (!t.Decode(host, s) || !t.IsValid()) return LC_ERR_PARSE;
        std::unique_ptr<lc_rgbt> h(new lc_rgbt());
        h->table = t;
        info[0] = t.Dimensions();
        info[1] = t.Divisions();
        info[2] = (uint32_t)t.Primaries();
        info[3] = (uint32_t)t.Gamma();
        info[4] = (uint32_t)t.Gamut();
        amount_range[0] = t.MinAmount();
        amount_range[1] = t.MaxAmount();
        *out = h.release();
        return LC_OK;
    });
}

void lc_rgb_table_free(lc_rgbt *t) { delete t; }

int lc_rgb_table_apply(const lc_rgbt *t, double amount, float *r, float *g, float *b, size_t n, int overrange) {
    if (!t || !r || !g || !b || n == 0 || n > 0x7fffffffu) return LC_ERR_BAD_ARG;
    return guarded([&] {
        dng_host host;
        dng_rgb_table table = t->table;
        table.SetAmount(amount);
        dng_rgb_to_rgb_table_data data(host, table);
        // one row of n pixels, three planes at the caller's (distinct) arrays
        std::vector<float> buf(n * 3);
        memcpy(buf.data(), r, n * sizeof(float));
        memcpy(buf.data() + n, g, n * sizeof(float));
        memcpy(buf.data() + 2 * n, b, n * sizeof(float));
        dng_pixel_buffer pb;
        pb.fArea = dng_rect(0, 0, 1, (int32)n);
        pb.fPlane = 0;
        pb.fPlanes = 3;
        pb.fRowStep = (int32)(n * 3);
        pb.fColStep = 1;
        pb.fPlaneStep = (int32)n;
        pb.fPixelType = ttFloat;
        pb.fPixelSize = 4;
        pb.fData = buf.data();
        data.Process_32(pb, nullptr, 0, pb.fArea, 0, overrange != 0);
        memcpy(r, buf.data(), n * sizeof(float));
        memcpy(g, buf.data() + n, n * sizeof(float));
        memcpy(b, buf.data() + 2 * n, n * sizeof(float));
        return LC_OK;
    });
}

int lc_render_dng(const uint8_t *dng, size_t dng_len, const uint8_t *dcp, size_t dcp_len, uint32_t max_size, int space,
                  uint32_t *w, uint32_t *h, float **out) {
    if (!dng || !w || !h || !out || dng_len < 16 || dng_len > 0x7fffffff) return LC_ERR_BAD_ARG;
    *out = nullptr;
    return guarded([&] {
        dng_stream stream(dng, (uint32)dng_len);
        dng_host host;
        AutoPtr<dng_negative> negative;
        {
            dng_info info;
            info.Parse(host, stream);
            info.PostParse(host);
            if (!info.IsValidDNG()) return LC_ERR_PARSE;
            negative.Reset(host.Make_dng_negative());
            negative->Parse(host, stream, info);
            negative->PostParse(host, stream, info);
            negative->ReadStage1Image(host, stream, info);
        }
        negative->SynchronizeMetadata();
        if (negative->Stage1Image()) negative->BuildStage2Image(host);
        if (negative->Stage2Image()) negative->BuildStage3Image(host);
        dng_camera_profile_id id;
        if (dcp && dcp_len >= 8 && dcp_len < 0x7fffffff) {
            AutoPtr<dng_camera_profile> prof(new dng_camera_profile());
            dng_stream ps(dcp, (uint32)dcp_len);
            if (!prof->ParseExtended(ps)) return LC_ERR_PARSE;
            id = prof->ProfileID();
            negative->AddProfile(prof);
        }
        dng_render render(host, *negative);
        if (id.IsValid()) render.SetCameraProfileID(id);
        render.SetFinalSpace(space == 1 ? dng_space_ProPhoto::Get() : dng_space_sRGB::Get());
        render.SetFinalPixelType(ttShort);
        if (max_size) render.SetMaximumSize(max_size);
        AutoPtr<dng_image> image(render.Render());
        if (!image.Get()) return LC_ERR_SDK;
        dng_rect bounds = image->Bounds();
        uint32 iw = bounds.W(), ih = bounds.H(), planes = image->Planes();
        if (planes != 3 || iw == 0 || ih == 0) return LC_ERR_SDK;
        dng_pixel_buffer buffer(bounds, 0, 3, ttShort, pcInterleaved, nullptr);
        AutoPtr<dng_memory_block> block(host.Allocate(buffer.fRowStep * ih * (uint32)sizeof(uint16)));
        buffer.fData = block->Buffer();
        image->Get(buffer);
        float *o = (float *)malloc((size_t)iw * ih * 3 * sizeof(float));
        if (!o) return LC_ERR_SDK;
        for (uint32 y = 0; y < ih; y++) {
            const uint16 *row = buffer.ConstPixel_uint16(bounds.t + y, bounds.l, 0);
            for (uint32 x = 0; x < iw * 3; x++) o[(size_t)y * iw * 3 + x] = row[x] / 65535.0f;
        }
        *w = iw;
        *h = ih;
        *out = o;
        return LC_OK;
    });
}

void lc_free(void *ptr) { free(ptr); }

}  // extern "C"
