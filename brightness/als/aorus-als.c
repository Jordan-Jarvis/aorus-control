// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * Ambient light sensor bridge for the tested GIGABYTE AERO 16 YE5.
 *
 * The firmware sends the current lux value through the AORUS WMI event GUID
 * as an ACPI buffer: { 0xf7, lux low, lux mid, lux high }.
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include <linux/acpi.h>
#include <linux/dmi.h>
#include <linux/iio/iio.h>
#include <linux/kernel.h>
#include <linux/module.h>
#include <linux/spinlock.h>
#include <linux/wmi.h>

#define AORUS_ALS_WMI_GUID "ABBC0F72-8EA1-11D1-00A0-C90629100000"
#define AORUS_ALS_EVENT_PREFIX 0xf7
#define AORUS_ALS_EVENT_LENGTH 4

struct aorus_als {
	spinlock_t lock;
	u32 lux;
	bool valid;
};

static const struct dmi_system_id aorus_als_dmi[] = {
	{
		.ident = "GIGABYTE AERO 16 YE5 (P86VE)",
		.matches = {
			DMI_EXACT_MATCH(DMI_SYS_VENDOR, "GIGABYTE"),
			DMI_EXACT_MATCH(DMI_PRODUCT_NAME, "AERO 16 YE5"),
			DMI_EXACT_MATCH(DMI_PRODUCT_VERSION, "P86VE"),
		},
	},
	{ }
};

static int aorus_als_read_raw(struct iio_dev *indio_dev,
				      const struct iio_chan_spec *chan,
				      int *val, int *val2, long mask)
{
	struct aorus_als *als = iio_priv(indio_dev);
	unsigned long flags;
	u32 lux;
	bool valid;

	if (chan->type != IIO_LIGHT || mask != IIO_CHAN_INFO_PROCESSED)
		return -EINVAL;

	spin_lock_irqsave(&als->lock, flags);
	lux = als->lux;
	valid = als->valid;
	spin_unlock_irqrestore(&als->lock, flags);

	if (!valid)
		return -ENODATA;

	*val = lux;
	*val2 = 0;
	return IIO_VAL_INT;
}

static const struct iio_info aorus_als_info = {
	.read_raw = aorus_als_read_raw,
};

static const struct iio_chan_spec aorus_als_channels[] = {
	{
		.type = IIO_LIGHT,
		.info_mask_separate = BIT(IIO_CHAN_INFO_PROCESSED),
	},
};

static void aorus_als_notify(struct wmi_device *wdev, union acpi_object *object)
{
	struct iio_dev *indio_dev = dev_get_drvdata(&wdev->dev);
	struct aorus_als *als;
	const u8 *data;
	u32 lux;
	unsigned long flags;

	if (!indio_dev)
		return;

	if (!object || object->type != ACPI_TYPE_BUFFER) {
		dev_warn_ratelimited(&wdev->dev,
			"ignoring WMI ALS event with non-buffer ACPI object\n");
		return;
	}

	if (object->buffer.length != AORUS_ALS_EVENT_LENGTH ||
	    !object->buffer.pointer ||
	    object->buffer.pointer[0] != AORUS_ALS_EVENT_PREFIX) {
		dev_warn_ratelimited(&wdev->dev,
			"ignoring malformed WMI ALS event (length=%u, prefix=%02x)\n",
			object->buffer.length,
			object->buffer.pointer ? object->buffer.pointer[0] : 0);
		return;
	}

	data = object->buffer.pointer;
	lux = data[1] | ((u32)data[2] << 8) | ((u32)data[3] << 16);
	als = iio_priv(indio_dev);

	spin_lock_irqsave(&als->lock, flags);
	als->lux = lux;
	als->valid = true;
	spin_unlock_irqrestore(&als->lock, flags);
}

static int aorus_als_probe(struct wmi_device *wdev, const void *context)
{
	struct iio_dev *indio_dev;
	struct aorus_als *als;
	int ret;

	indio_dev = iio_device_alloc(&wdev->dev, sizeof(*als));
	if (!indio_dev)
		return -ENOMEM;

	als = iio_priv(indio_dev);
	spin_lock_init(&als->lock);

	indio_dev->name = "aorus-ambient-light";
	indio_dev->info = &aorus_als_info;
	indio_dev->modes = INDIO_DIRECT_MODE;
	indio_dev->channels = aorus_als_channels;
	indio_dev->num_channels = ARRAY_SIZE(aorus_als_channels);
	dev_set_drvdata(&wdev->dev, indio_dev);

	ret = iio_device_register(indio_dev);
	if (ret) {
		dev_set_drvdata(&wdev->dev, NULL);
		iio_device_free(indio_dev);
		return ret;
	}

	return 0;
}

static void aorus_als_remove(struct wmi_device *wdev)
{
	struct iio_dev *indio_dev = dev_get_drvdata(&wdev->dev);

	if (!indio_dev)
		return;

	dev_set_drvdata(&wdev->dev, NULL);
	iio_device_unregister(indio_dev);
	iio_device_free(indio_dev);
}

static const struct wmi_device_id aorus_als_wmi_ids[] = {
	{ AORUS_ALS_WMI_GUID, NULL },
	{ }
};
MODULE_DEVICE_TABLE(wmi, aorus_als_wmi_ids);

static struct wmi_driver aorus_als_driver = {
	.driver = {
		.name = "aorus-als",
	},
	.id_table = aorus_als_wmi_ids,
	.probe = aorus_als_probe,
	.remove = aorus_als_remove,
	.notify = aorus_als_notify,
};

static int __init aorus_als_init(void)
{
	if (!dmi_check_system(aorus_als_dmi)) {
		pr_info("unsupported DMI identity; not loading\n");
		return -ENODEV;
	}

	if (!wmi_has_guid(AORUS_ALS_WMI_GUID)) {
		pr_info("ALS WMI GUID not present; not loading\n");
		return -ENODEV;
	}

	return wmi_driver_register(&aorus_als_driver);
}

static void __exit aorus_als_exit(void)
{
	wmi_driver_unregister(&aorus_als_driver);
}

module_init(aorus_als_init);
module_exit(aorus_als_exit);

MODULE_AUTHOR("AORUS Control contributors");
MODULE_DESCRIPTION("GIGABYTE AERO 16 WMI ambient light sensor");
MODULE_LICENSE("GPL");
