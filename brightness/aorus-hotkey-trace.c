// SPDX-License-Identifier: GPL-2.0-or-later
#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include <linux/acpi.h>
#include <linux/errno.h>
#include <linux/module.h>

#define AORUS_WMI_EVENT_GUID "ABBC0F72-8EA1-11D1-00A0-C90629100000"

static void aorus_hotkey_trace_notify(union acpi_object *object, void *context)
{
	if (object && object->type == ACPI_TYPE_INTEGER)
		pr_info("WMI event code=0x%llx\n", object->integer.value);
	else if (object && object->type == ACPI_TYPE_BUFFER) {
		u32 length = min_t(u32, object->buffer.length, 64);

		pr_info("WMI event buffer length=%u data=%*phN%s\n",
			object->buffer.length, length, object->buffer.pointer,
			object->buffer.length > length ? "..." : "");
	}
	else if (object)
		pr_info("WMI event object type=%u\n", object->type);
	else
		pr_info("WMI event without data\n");
}

static int __init aorus_hotkey_trace_init(void)
{
	acpi_status status;

	if (!wmi_has_guid(AORUS_WMI_EVENT_GUID))
		return -ENODEV;
	status = wmi_install_notify_handler(AORUS_WMI_EVENT_GUID,
					    aorus_hotkey_trace_notify, NULL);
	if (ACPI_FAILURE(status)) {
		pr_err("failed to register WMI handler: %s\n",
		       acpi_format_exception(status));
		return -EIO;
	}
	pr_info("capturing WMI events\n");
	return 0;
}

static void __exit aorus_hotkey_trace_exit(void)
{
	wmi_remove_notify_handler(AORUS_WMI_EVENT_GUID);
	pr_info("capture stopped\n");
}

module_init(aorus_hotkey_trace_init);
module_exit(aorus_hotkey_trace_exit);
MODULE_DESCRIPTION("Temporary Gigabyte AERO/AORUS WMI hotkey tracer");
MODULE_AUTHOR("AORUS Control contributors");
MODULE_LICENSE("GPL");
