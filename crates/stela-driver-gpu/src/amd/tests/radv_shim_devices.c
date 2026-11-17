/* SPDX-License-Identifier: GPL-2.0-or-later */
/* The device table of Mesa's amdgpu no-op DRM shim for `cargo xtask gpu
 * radv-test`, linked in place of the shim's own, with the shim's start
 * compiled as stela_mesa_drm_shim_driver_init: the shim's 20 recorded
 * devices, then one for each chip RADV supports that has no record, made
 * from a recorded sibling (`radv_shim_derived.h`, which `radv/devices.rs`
 * writes from Mesa's and amdgpu's tables). A derived device keeps its
 * sibling's queues, registers, firmware and memory, and takes its chip's
 * family, revision, PCI ID, APU flag and, where its generation differs, its
 * graphics IP version: what Mesa identifies a chip and its generation by. */
#define amdgpu_devices stela_mesa_amdgpu_devices
#define num_amdgpu_devices stela_mesa_num_amdgpu_devices
#include "common/amdgpu_devices.c"
#undef amdgpu_devices
#undef num_amdgpu_devices

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct stela_derived_device {
	const char *name, *base;
	enum radeon_family chip;
	uint32_t family, external_rev, device_id;
	/* The graphics IP version, zero to keep the sibling's. */
	uint32_t major, minor;
	uint32_t apu;
};

#include "radv_shim_derived.h"

#define DEVICES 128

/* The table the shim reads, which amdgpu_devices.h declares constant; it
 * is filled where the shim starts its device, before it looks one up. */
struct amdgpu_device amdgpu_devices[DEVICES];
size_t num_amdgpu_devices;

/* The shim's own start (amdgpu_noop_drm_shim.c), compiled under this name. */
void stela_mesa_drm_shim_driver_init(void);
void drm_shim_driver_init(void);

static void set_version(struct drm_amdgpu_info_hw_ip *ip, uint32_t major, uint32_t minor)
{
	/* Mesa reads the discovery version where the kernel gives one. */
	if (ip->ip_discovery_version)
		ip->ip_discovery_version = major << 16 | minor << 8;
	ip->hw_ip_version_major = major;
	ip->hw_ip_version_minor = minor;
}

static void fill(void)
{
	size_t count = stela_mesa_num_amdgpu_devices;
	size_t derived = sizeof(stela_derived) / sizeof(stela_derived[0]);
	if (count + derived > DEVICES) {
		fprintf(stderr, "radv-test: %zu shim devices exceed the table's %d\n",
			count + derived, DEVICES);
		abort();
	}
	memcpy(amdgpu_devices, stela_mesa_amdgpu_devices, count * sizeof(amdgpu_devices[0]));
	for (size_t i = 0; i < derived; i++) {
		const struct stela_derived_device *spec = &stela_derived[i];
		const struct amdgpu_device *base = NULL;
		for (size_t j = 0; j < stela_mesa_num_amdgpu_devices && !base; j++) {
			if (!strcmp(stela_mesa_amdgpu_devices[j].name, spec->base))
				base = &stela_mesa_amdgpu_devices[j];
		}
		if (!base) {
			fprintf(stderr, "radv-test: no shim record %s for %s\n", spec->base, spec->name);
			abort();
		}
		struct amdgpu_device *device = &amdgpu_devices[count++];
		*device = *base;
		device->name = spec->name;
		device->radeon_family = spec->chip;
		device->dev.family = spec->family;
		device->dev.external_rev = spec->external_rev;
		device->dev.device_id = spec->device_id;
		device->dev.ids_flags = (device->dev.ids_flags & ~(uint64_t)AMDGPU_IDS_FLAGS_FUSION) |
					(spec->apu ? AMDGPU_IDS_FLAGS_FUSION : 0);
		if (spec->major) {
			set_version(&device->hw_ip_gfx, spec->major, spec->minor);
			set_version(&device->hw_ip_compute, spec->major, spec->minor);
		}
	}
	num_amdgpu_devices = count;
}

/* The shim starts its device on the first call it interposes, which other
 * libraries' initializers may make before this library's own run. */
void drm_shim_driver_init(void)
{
	if (!num_amdgpu_devices)
		fill();
	stela_mesa_drm_shim_driver_init();
}
