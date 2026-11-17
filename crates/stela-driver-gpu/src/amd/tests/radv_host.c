/* SPDX-License-Identifier: GPL-2.0-or-later */
/* The parts of the image's Linux side and of a render pipeline a hosted RADV
 * library needs beside RADV, the AIR compiler and Stela's libdrm stand-in,
 * when a host process opens the Metal device over it (`cargo xtask gpu
 * radv-test`): the one render node Mesa's amdgpu no-op DRM shim shows, and
 * a vertex stage that feeds a fragment function compiled alone, so that a
 * graphics pipeline RADV builds holds it. */
#include <string.h>

#include <stela/host.h>

#include "nir_builder.h"

/* The PCI function the hosted device sits at, as the opener names it. RADV
 * takes the device's own IDs from amdgpu's queries, not from this. */
#define HOST_BUS 4

unsigned int stela_libc_drm_devices(void);
void stela_libc_drm_device(unsigned int index, struct stela_libc_drm_device *device);
nir_shader *stela_radv_host_feeding_vertex(const nir_shader *fragment,
					   const nir_shader_compiler_options *options);

unsigned int stela_libc_drm_devices(void)
{
	return 1;
}

void stela_libc_drm_device(unsigned int index, struct stela_libc_drm_device *device)
{
	memset(device, 0, sizeof(*device));
	if (index != 0)
		return;
	device->vendor = 0x1002;
	device->device = 0x15e7;
	device->bus = HOST_BUS;
}

/* Stores zero to every element of `deref`, of any type. */
static void store_zero(nir_builder *b, nir_deref_instr *deref)
{
	const struct glsl_type *type = deref->type;
	if (glsl_type_is_vector_or_scalar(type)) {
		unsigned components = glsl_get_vector_elements(type);
		unsigned bits = glsl_get_bit_size(type);
		nir_store_deref(b, deref, nir_imm_zero(b, components, bits ? bits : 32),
				nir_component_mask(components));
	} else if (glsl_type_is_array_or_matrix(type)) {
		for (unsigned i = 0; i < glsl_get_length(type); i++)
			store_zero(b, nir_build_deref_array_imm(b, deref, i));
	} else if (glsl_type_is_struct(type)) {
		for (unsigned i = 0; i < glsl_get_length(type); i++)
			store_zero(b, nir_build_deref_struct(b, deref, i));
	}
}

/* A vertex stage that writes a position and zero to each of `fragment`'s
 * varying inputs, at their locations, with their types and interpolation:
 * the stage a pipeline needs before a fragment function compiled with its
 * own inputs as the linked varyings. The fragment function's code is what
 * the pipeline is built for; this stage only completes it. */
nir_shader *stela_radv_host_feeding_vertex(const nir_shader *fragment,
					   const nir_shader_compiler_options *options)
{
	nir_builder b =
		nir_builder_init_simple_shader(MESA_SHADER_VERTEX, options, "stela_feeding_vertex");
	nir_variable *position =
		nir_variable_create(b.shader, nir_var_shader_out, glsl_vec4_type(), "position");
	position->data.location = VARYING_SLOT_POS;
	nir_store_var(&b, position, nir_imm_vec4(&b, 0.0f, 0.0f, 0.0f, 1.0f), 0xf);
	nir_foreach_shader_in_variable(input, fragment) {
		/* Fragment coordinates, facing, sample and primitive IDs and
		 * the like are the rasterizer's, not the vertex stage's. */
		if (input->data.location < VARYING_SLOT_VAR0 ||
		    input->data.location >= VARYING_SLOT_VAR0_16BIT)
			continue;
		nir_variable *output = nir_variable_clone(input, b.shader);
		output->data.mode = nir_var_shader_out;
		nir_shader_add_variable(b.shader, output);
		store_zero(&b, nir_build_deref_var(&b, output));
	}
	return b.shader;
}
