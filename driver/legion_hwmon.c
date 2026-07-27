// SPDX-License-Identifier: GPL-2.0-only
/*
 * legion_hwmon.c — Lenovo Legion EC hardware monitor (via port I/O).
 *
 * Reads EC CPU/GPU temperatures via ports 0x62/0x66 (ACPI EC protocol).
 * Uses the same protocol as ec_sys but from kernel space. Read-only.
 *
 * Spectrum keyboard RGB (ITE 048d:c197) is USB HID — panic detection and
 * auto-fix live in userspace (legion-daemon rgb-watchdog + rgb_panic),
 * which scans kernel USB/HID logs and resets the USB device when needed.
 *
 * Exposed via hwmon:
 *   temp1_input — EC CPU temperature (°C, register 0xB0)
 *   temp2_input — EC GPU temperature (°C, register 0xB4)
 *
 * DKMS auto-rebuilds on every kernel update.
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/hwmon.h>
#include <linux/err.h>
#include <linux/io.h>
#include <linux/delay.h>
#include <linux/platform_device.h>

#define EC_DATA_PORT  0x62
#define EC_CMD_PORT   0x66
#define EC_READ_CMD   0x80

#define EC_CPU_TEMP   0xB0
#define EC_GPU_TEMP   0xB4

/* Standard ACPI EC read protocol */
static int ec_read_byte(u8 addr, u8 *val)
{
    int retries;
    u8 status;

    /* Wait for IBF clear */
    for (retries = 0; retries < 1000; retries++) {
        status = inb(EC_CMD_PORT);
        if (!(status & 0x02))
            break;
        udelay(10);
    }
    if (retries >= 1000)
        return -ETIMEDOUT;

    /* Send read command */
    outb(EC_READ_CMD, EC_CMD_PORT);

    /* Wait for IBF clear */
    for (retries = 0; retries < 1000; retries++) {
        status = inb(EC_CMD_PORT);
        if (!(status & 0x02))
            break;
        udelay(10);
    }
    if (retries >= 1000)
        return -ETIMEDOUT;

    /* Send address */
    outb(addr, EC_DATA_PORT);

    /* Wait for OBF set */
    for (retries = 0; retries < 1000; retries++) {
        status = inb(EC_CMD_PORT);
        if (status & 0x01)
            break;
        udelay(10);
    }
    if (retries >= 1000)
        return -ETIMEDOUT;

    /* Read data */
    *val = inb(EC_DATA_PORT);
    return 0;
}

/* ─── hwmon interface ─── */

static int legion_hwmon_read(struct device *dev, enum hwmon_sensor_types type,
                              u32 attr, int channel, long *val)
{
    u8 temp;
    int ret;

    if (type != hwmon_temp || attr != hwmon_temp_input)
        return -EOPNOTSUPP;

    switch (channel) {
    case 0:
        ret = ec_read_byte(EC_CPU_TEMP, &temp);
        break;
    case 1:
        ret = ec_read_byte(EC_GPU_TEMP, &temp);
        break;
    default:
        return -EOPNOTSUPP;
    }

    if (ret < 0)
        return ret;

    *val = (long)temp * 1000;
    return 0;
}

static int legion_hwmon_read_string(struct device *dev,
                                     enum hwmon_sensor_types type,
                                     u32 attr, int channel, const char **str)
{
    if (type != hwmon_temp || attr != hwmon_temp_label)
        return -EOPNOTSUPP;

    switch (channel) {
    case 0: *str = "EC CPU"; break;
    case 1: *str = "EC GPU"; break;
    default: return -EOPNOTSUPP;
    }
    return 0;
}

static umode_t legion_hwmon_is_visible(const void *data,
                                        enum hwmon_sensor_types type,
                                        u32 attr, int channel)
{
    if (type != hwmon_temp)
        return 0;
    if (channel > 1)
        return 0;
    return 0444;
}

static const struct hwmon_channel_info * const legion_hwmon_info[] = {
    HWMON_CHANNEL_INFO(temp,
        HWMON_T_INPUT | HWMON_T_LABEL,
        HWMON_T_INPUT | HWMON_T_LABEL
    ),
    NULL
};

static const struct hwmon_ops legion_hwmon_ops = {
    .is_visible  = legion_hwmon_is_visible,
    .read        = legion_hwmon_read,
    .read_string = legion_hwmon_read_string,
};

static const struct hwmon_chip_info legion_hwmon_chip = {
    .ops  = &legion_hwmon_ops,
    .info = legion_hwmon_info,
};

/* ─── init / exit ─── */

static struct platform_device *pdev;
static struct device *hwmon_dev;

static int __init legion_hwmon_init(void)
{
    u8 test;
    int ret;

    pr_info("legion_hwmon: probing EC at ports 0x%02X/0x%02X\n",
            EC_CMD_PORT, EC_DATA_PORT);

    ret = ec_read_byte(EC_CPU_TEMP, &test);
    if (ret < 0) {
        pr_err("legion_hwmon: EC not responding (error %d)\n", ret);
        return -ENODEV;
    }
    pr_info("legion_hwmon: EC CPU temp = %u°C (sanity read OK)\n", test);

    /* Register a dummy platform device so hwmon has a parent */
    pdev = platform_device_register_simple("legion_hwmon", -1, NULL, 0);
    if (IS_ERR(pdev)) {
        pr_err("legion_hwmon: platform device failed\n");
        return PTR_ERR(pdev);
    }

    hwmon_dev = devm_hwmon_device_register_with_info(
        &pdev->dev, "legion_hwmon", NULL, &legion_hwmon_chip, NULL);

    if (IS_ERR(hwmon_dev)) {
        pr_err("legion_hwmon: hwmon registration failed: %ld\n",
               PTR_ERR(hwmon_dev));
        platform_device_unregister(pdev);
        return PTR_ERR(hwmon_dev);
    }

    pr_info("legion_hwmon: ready — sensors legion_hwmon-*\n");
    return 0;
}

static void __exit legion_hwmon_exit(void)
{
    if (pdev)
        platform_device_unregister(pdev);
    pr_info("legion_hwmon: unloaded\n");
}

module_init(legion_hwmon_init);
module_exit(legion_hwmon_exit);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("Lenovo Legion Tool");
MODULE_DESCRIPTION("Lenovo Legion EC hwmon — CPU/GPU temperatures");
