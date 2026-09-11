// SPDX-License-Identifier: GPL-2.0-only

#include "vmlinux.h"
#include "hid_bpf.h"
#include "hid_bpf_helpers.h"
#include <bpf/bpf_tracing.h>

#define VID_GIGABYTE 0x1044
#define PID_AERO_KEYBOARD 0x7a3a
#define ORIGINAL_RDESC_SIZE 253
#define FIXED_RDESC_SIZE (ORIGINAL_RDESC_SIZE + sizeof(translated_rdesc))
#define VENDOR_REPORT_ID 0x04
#define ACTION_MAP_VERSION 2

/* Versioned user-to-kernel action map. A zero report_id is an explicit
 * disabled mapping; an all-zero value means the daemon has not configured the
 * map yet and the built-in defaults remain active. */
struct fn_action_value {
	__u8 version;
	__u8 report_id;
	__u16 payload;
	__u32 generation;
};
struct {
	__uint(type, BPF_MAP_TYPE_ARRAY);
	__uint(max_entries, 8);
	__type(key, __u32);
	__type(value, struct fn_action_value);
} aorus_fn_act_v2 SEC(".maps");

HID_BPF_CONFIG(
	HID_DEVICE(BUS_USB, HID_GROUP_GENERIC, VID_GIGABYTE, PID_AERO_KEYBOARD)
);

/* Exact descriptor from USB interface 2 on the AERO 16 YE5 (P86VE). */
static const __u8 original_rdesc[ORIGINAL_RDESC_SIZE] = {
	0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x01, 0x09, 0x01, 0xa1, 0x00,
	0x05, 0x09, 0x15, 0x00, 0x25, 0x01, 0x19, 0x01, 0x29, 0x05, 0x75, 0x01,
	0x95, 0x05, 0x81, 0x02, 0x95, 0x03, 0x81, 0x01, 0x05, 0x01, 0x16, 0x01,
	0x80, 0x26, 0xff, 0x7f, 0x09, 0x30, 0x09, 0x31, 0x75, 0x10, 0x95, 0x02,
	0x81, 0x06, 0x15, 0x81, 0x25, 0x7f, 0x09, 0x38, 0x75, 0x08, 0x95, 0x01,
	0x81, 0x06, 0x05, 0x0c, 0x0a, 0x38, 0x02, 0x95, 0x01, 0x81, 0x06, 0xc0,
	0xc0, 0x05, 0x01, 0x09, 0x80, 0xa1, 0x01, 0x85, 0x02, 0x19, 0x81, 0x29,
	0x83, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x03, 0x81, 0x02, 0x95,
	0x05, 0x81, 0x01, 0xc0, 0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x03,
	0x19, 0x00, 0x2a, 0xff, 0x07, 0x15, 0x00, 0x26, 0xff, 0x07, 0x95, 0x01,
	0x75, 0x10, 0x81, 0x00, 0xc0, 0x06, 0x02, 0xff, 0x09, 0x01, 0xa1, 0x01,
	0x85, 0x04, 0x15, 0x00, 0x26, 0xff, 0x00, 0x09, 0x03, 0x75, 0x08, 0x95,
	0x03, 0x81, 0x00, 0xc0, 0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x05,
	0x05, 0x07, 0x95, 0x01, 0x75, 0x08, 0x81, 0x03, 0x95, 0xe8, 0x75, 0x01,
	0x15, 0x00, 0x25, 0x01, 0x05, 0x07, 0x19, 0x00, 0x29, 0xe7, 0x81, 0x00,
	0xc0, 0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x06, 0x15, 0x00, 0x26,
	0xff, 0x7f, 0x09, 0x30, 0x09, 0x31, 0x75, 0x10, 0x95, 0x02, 0x81, 0x02,
	0xc0, 0x06, 0x89, 0xff, 0x09, 0x10, 0xa1, 0x01, 0x85, 0x5a, 0x09, 0x01,
	0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x10, 0xb1, 0x00, 0xc0,
	0x05, 0x01, 0x09, 0x0c, 0xa1, 0x01, 0x85, 0x07, 0x15, 0x00, 0x25, 0x01,
	0x09, 0xc6, 0x95, 0x01, 0x75, 0x01, 0x81, 0x06, 0x75, 0x07, 0x81, 0x03,
	0xc0,
};

/*
 * Every report ID below is new, so the original report collections stay
 * intact. The translated keyboard collection is reserved for daemon actions;
 * standard actions use the native HID usages in their own collections.
 * Display is on another HID interface and touchpad lock emits a native
 * interface-0 chord, so neither duplicate interface-2 report is translated.
 *
 * The relative fields are deliberate. For brightness, fan, Wi-Fi, square-X,
 * and AI, the capture proves one press report but no release report. Relative
 * HID semantics supply native pulse/repeat behavior without inventing release
 * state or a userspace repeat timer. Sleep is different: both 02 02 and 02
 * 00 were captured, so its field is absolute and preserves that exact
 * press/release pair while suppressing the original System Sleep report.
 */
static const __u8 translated_rdesc[] = {
	0x05, 0x07,       /* Usage Page (Keyboard/Keypad) */
	0x09, 0x06,       /* Usage (Keyboard) */
	0xa1, 0x01,       /* Collection (Application) */
	0x15, 0x00,       /* Logical Minimum (0) */
	0x25, 0x01,       /* Logical Maximum (1) */

	/* Report ID 8: ten reserved daemon-action identities, each a pulse. */
	0x85, 0x08,
	0x09, 0x68,       /* F13: 04 00 00 7d, brightness down */
	0x09, 0x69,       /* F14: 04 00 00 7e, brightness up */
	0x09, 0x6a,       /* reserved daemon action: 04 00 00 84, fan */
	0x09, 0x6b,       /* F16: daemon action */
	0x09, 0x6c,       /* F17: 04 00 00 7c, Wi-Fi */
	0x09, 0x6d,       /* F18: daemon action */
	0x09, 0x6e,       /* reserved daemon action: 04 00 00 80, square-X */
	0x09, 0x6f,       /* F20: daemon action */
	0x09, 0x70,       /* F21: daemon action */
	0x09, 0x71,       /* reserved daemon action: 04 00 00 88, AI */
	0x75, 0x01, 0x95, 0x0a,
	0x81, 0x06,       /* Input (Data, Variable, Relative) */
	0x75, 0x06, 0x95, 0x01, 0x81, 0x03,

	0xc0,             /* End Collection */

	/* Report ID 10: native HID consumer/media pulses. */
	0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01,
	0x15, 0x00, 0x25, 0x01, 0x85, 0x0a,
	0x09, 0x6f, 0x09, 0x70, 0x09, 0x32, 0x09, 0xe2,
	0x09, 0xe9, 0x09, 0xea, 0x09, 0xcd,
	0x75, 0x01, 0x95, 0x07, 0x81, 0x06,
	0x75, 0x01, 0x95, 0x01, 0x81, 0x03, 0xc0,

	/* Report ID 11: native Wireless Radio Controls / RFKill button. */
	0x05, 0x01, 0x09, 0x0c, 0xa1, 0x01,
	0x15, 0x00, 0x25, 0x01, 0x85, 0x0b, 0x09, 0xc6,
	0x75, 0x01, 0x95, 0x01, 0x81, 0x06,
	0x75, 0x07, 0x95, 0x01, 0x81, 0x03, 0xc0,

	/* Report ID 12: native keyboard Print Screen pulse. */
	0x05, 0x07, 0x09, 0x06, 0xa1, 0x01,
	0x15, 0x00, 0x25, 0x01, 0x85, 0x0c, 0x09, 0x46,
	0x75, 0x01, 0x95, 0x01, 0x81, 0x06,
	0x75, 0x07, 0x95, 0x01, 0x81, 0x03, 0xc0,
};
static __always_inline bool descriptor_matches(const __u8 *descriptor)
{
	int i;

	for (i = 0; i < ORIGINAL_RDESC_SIZE; i++)
		if (descriptor[i] != original_rdesc[i])
			return false;

	return true;
}

static __always_inline struct fn_action_value *configured_action(__u32 key)
{
	return bpf_map_lookup_elem(&aorus_fn_act_v2, &key);
}

static __always_inline int emit_action(__u8 *data,
				       const struct fn_action_value *action,
				       bool pressed)
{
	data[0] = action->report_id;
	data[1] = pressed ? action->payload & 0xff : 0;
	if (action->report_id == 0x08) {
		data[2] = pressed ? action->payload >> 8 : 0;
		return 3;
	}
	return 2;
}

static __always_inline int emit_disabled(__u8 *data, bool sleep_report)
{
	data[0] = sleep_report ? 0x0a : 0x08;
	data[1] = 0;
	if (!sleep_report)
		data[2] = 0;
	return sleep_report ? 2 : 3;
}

SEC(HID_BPF_RDESC_FIXUP)
int BPF_PROG(aero_16_ye5_fix_rdesc, struct hid_bpf_ctx *hctx)
{
	__u8 *data;

	if (hctx->size != ORIGINAL_RDESC_SIZE)
		return 0;

	data = hid_bpf_get_data(hctx, 0, HID_MAX_DESCRIPTOR_SIZE);
	if (!data || !descriptor_matches(data))
		return 0;

	__builtin_memcpy(data + ORIGINAL_RDESC_SIZE, translated_rdesc,
			 sizeof(translated_rdesc));
	return FIXED_RDESC_SIZE;
}

SEC(HID_BPF_DEVICE_EVENT)
int BPF_PROG(aero_16_ye5_fix_event, struct hid_bpf_ctx *hctx,
	     enum hid_report_type type)
{
	__u8 *data;
	__u8 key_mask;

	if (type != HID_INPUT_REPORT)
		return 0;

	/* Only the seven capture-proven interface-2 vendor reports are translated. */
	if (hctx->size == 4) {
		data = hid_bpf_get_data(hctx, 0, 4);
		if (!data || data[0] != VENDOR_REPORT_ID || data[1] || data[2])
			return 0;

		/* Each report becomes a one-byte, modifierless F-key pulse. */
		if (data[3] == 0x7d) {
			__u32 key = 0;
			struct fn_action_value *action = configured_action(key);
			if (action && action->version == ACTION_MAP_VERSION) {
				return action->report_id ? emit_action(data, action, true) :
					emit_disabled(data, false);
			}
			data[0] = 0x0a;
			data[1] = 0x02; /* consumer Brightness Down (usage 0x70) */
			return 2;
		} else if (data[3] == 0x7e) {
			__u32 key = 1;
			struct fn_action_value *action = configured_action(key);
			if (action && action->version == ACTION_MAP_VERSION) {
				return action->report_id ? emit_action(data, action, true) :
					emit_disabled(data, false);
			}
			data[0] = 0x0a;
			data[1] = 0x01; /* consumer Brightness Up (usage 0x6f) */
			return 2;
		} else if (data[3] == 0x84) {
			key_mask = 0x04;
		} else if (data[3] == 0x81) {
			return 0; /* interface 0 supplies the native touchpad toggle */
		} else if (data[3] == 0x7c) {
			key_mask = 0x08;
		} else if (data[3] == 0x80) {
			key_mask = 0x10;
		} else if (data[3] == 0x88) {
			key_mask = 0x20;
		} else {
			/* Includes the unmodified airplane report 07 01. */
			return 0;
		}

		{
			__u32 key = key_mask == 0x04 ? 2 : key_mask == 0x08 ? 4 : key_mask == 0x10 ? 5 : 6;
			struct fn_action_value *action = configured_action(key);
			if (action && action->version == ACTION_MAP_VERSION) {
				return action->report_id ? emit_action(data, action, true) :
					emit_disabled(data, false);
			}
			data[0] = 0x08;
			data[1] = key_mask;
			data[2] = 0;
			return 3;
		}
	}

	/* The captured sleep pair is translated to a native consumer pulse. */
	if (hctx->size == 2) {
		/* The fixed descriptor's largest input report is three bytes (the
		 * private-action report). Request that full buffer before allowing a
		 * remapped sleep key to use it; requesting only the source length
		 * makes the verifier correctly reject the possible third-byte write. */
		data = hid_bpf_get_data(hctx, 0, 3);
		if (!data || data[0] != 0x02 ||
		    (data[1] != 0x02 && data[1] != 0x00))
			return 0;

		{
			__u32 key = 3;
			struct fn_action_value *action = configured_action(key);
			if (action && action->version == ACTION_MAP_VERSION)
				return action->report_id ? emit_action(data, action,
									data[1] == 0x02) :
					emit_disabled(data, true);
		}
		data[0] = 0x0a;
		data[1] = data[1] == 0x02 ? 0x04 : 0;
		return 2;
	}

	return 0;
}

HID_BPF_OPS(aero_16_ye5_brightness) = {
	.hid_device_event = (void *)aero_16_ye5_fix_event,
	.hid_rdesc_fixup = (void *)aero_16_ye5_fix_rdesc,
};

SEC("syscall")
int probe(struct hid_bpf_probe_args *ctx)
{
	/*
	 * The privileged udev helper performs the exact DMI, HID ID, interface,
	 * and descriptor checks before loading this object.  Keeping this probe
	 * side-effect-free is intentional: kernels reject variable pointer walks
	 * through hid_bpf_probe_args->rdesc even though the fixed-up callback can
	 * safely access the descriptor with hid_bpf_get_data().
	 */
	ctx->retval = 0;
	return 0;
}

char _license[] SEC("license") = "GPL";
