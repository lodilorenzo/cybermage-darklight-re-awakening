// C ABI for the vendored GPL-2.0-or-later Nuked-SC55 backend.
#include "emu.h"
#include "file_hashing.h"
#include <algorithm>
#include <cstring>
#include <deque>
#include <memory>
#include <stdexcept>

struct SC55 {
    RomsetInfo roms; // Must outlive Emulator.
    Emulator emu;
    std::deque<AudioFrame<float>> samples;
    AudioFrame<float> a{}, b{};
    double phase = 0;
    double source_rate = 0;

    static void sample(void* user, const AudioFrame<int32_t>& frame) {
        auto& s = *static_cast<SC55*>(user);
        AudioFrame<float> normalized;
        Normalize(frame, normalized);
        s.samples.push_back(normalized);
    }
    AudioFrame<float> next() {
        while (samples.empty()) emu.Step();
        auto frame = samples.front();
        samples.pop_front();
        return frame;
    }
};

extern "C" {
void* cm_sc55_open(const char* directory, char* error, size_t error_size) noexcept {
    try {
        auto s = std::make_unique<SC55>();
        HashedFileRegistry files;
        if (!HashDirectoryFiles(std::filesystem::u8path(directory), HashDirectoryKind::Recursive, files,
            [](const auto& f) { return f.file_size() <= ROM_MAX_SIZE; })) {
            throw std::runtime_error("Cannot read SC-55 ROM directory");
        }
        auto registry = RomsetRegistry::CreateWithDefaultHashes();
        // Deterministic preference: original SC-55, then SC-55mkII; first registered revision.
        bool loaded = false;
        for (auto family : {Romset::MK1, Romset::MK2}) {
            for (const auto& def : registry) {
                if (def.romset != family ||
                    !ContainsRomsetFiles(registry, def.name, files, ROMLOCATION_ALL)) continue;
                GetRomsetInfo(registry, def.name, files, ROMLOCATION_ALL, s->roms);
                if (!LoadRomset(s->roms, nullptr) || !s->emu.Init({}) ||
                    !s->emu.LoadRoms(family, s->roms)) {
                    throw std::runtime_error("Cannot load SC-55 ROM set");
                }
                loaded = true;
                break;
            }
            if (loaded) break;
        }
        if (!loaded) throw std::runtime_error("No complete supported SC-55 / SC-55mkII ROM set");
        s->emu.Reset();
        // Match upstream's offline renderer: allow firmware boot and GM reset to settle.
        s->emu.PostSystemReset(EMU_SystemReset::GM_RESET);
        for (size_t i = 0; i < 24'000'000; ++i) s->emu.Step();
        s->source_rate = PCM_GetOutputFrequency(s->emu.GetPCM());
        s->emu.SetSampleCallback(SC55::sample, s.get());
        s->a = s->next();
        s->b = s->next();
        return s.release();
    } catch (const std::exception& e) {
        if (error_size) {
            std::strncpy(error, e.what(), error_size - 1);
            error[error_size - 1] = 0;
        }
        return nullptr;
    } catch (...) {
        if (error_size) error[0] = 0;
        return nullptr;
    }
}
void cm_sc55_close(void* handle) noexcept { delete static_cast<SC55*>(handle); }
void cm_sc55_send(void* handle, const uint8_t* data, size_t size) noexcept {
    static_cast<SC55*>(handle)->emu.PostMIDI(std::span(data, size));
}
bool cm_sc55_mix(void* handle, float* stereo, size_t frames, double rate) noexcept {
    try {
        auto& s = *static_cast<SC55*>(handle);
        // ponytail: linear resampling; replace with a band-limited resampler if aliasing is audible.
        const double step = s.source_rate / rate;
        for (size_t i = 0; i < frames; ++i) {
            stereo[i*2] += s.a.left + (s.b.left-s.a.left) * static_cast<float>(s.phase);
            stereo[i*2+1] += s.a.right + (s.b.right-s.a.right) * static_cast<float>(s.phase);
            s.phase += step;
            while (s.phase >= 1) {
                s.phase -= 1;
                s.a = s.b;
                s.b = s.next();
            }
        }
        return true;
    } catch (...) { return false; } // Never unwind across Rust FFI (e.g. allocation failure).
}
}
