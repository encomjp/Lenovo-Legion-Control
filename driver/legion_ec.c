/*
 * legion_ec.c — Minimal PoC kernel module to read Lenovo Legion EC RAM.
 *
 * Maps the EC memory at physical 0xFE00D400 via ioremap and exposes
 * sensor data via debugfs. Read-only, no writes to EC.
 *
 * Build: make
 * Test:  sudo insmod legion_ec.ko && cat /sys/kernel/debug/legion_ec/sensors
 *
 * If this returns valid data, the extended EC RAM is accessible and
 * the register offsets from LLL are correct for this model.
 * If it returns zeros or garbage, Gen 10 (IT5508) needs WMI3 instead.
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/io.h>
#include <linux/debugfs.h>
#include <linux/uaccess.h>

#define EC_PHYS_BASE   0xFE00D400
#define EC_MAP_SIZE    0x1000

/* EC register offsets (from LenovoLegionLinux ec_register_offsets_v0) */
#define EC_CHIP_ID1     0x2000
#define EC_CHIP_ID2     0x2001
#define EC_CHIP_VER     0x2002
#define EC_FW_VER       0xC2C7

#define EC_CPU_TEMP     0xC580
#define EC_CPU_TEMP_ALT 0xC538
#define EC_GPU_TEMP     0xC5A0
#define EC_GPU_TEMP_ALT 0xC539
#define EC_VRM_TEMP     0xC5C0
#define EC_IC_TEMP      0xC5E8

#define EC_FAN1_RPM_LO  0xC5E0
#define EC_FAN1_RPM_HI  0xC5E1
#define EC_FAN2_RPM_LO  0xC5E2
#define EC_FAN2_RPM_HI  0xC5E3

#define EC_POWERMODE     0xC420
#define EC_LOCKFAN       0xC4AB

#define EC_FAN_BASE      0xC534
#define EC_FAN1_BASE     0xC540
#define EC_FAN2_BASE     0xC550

static void __iomem *ec_ram;
static struct dentry *debugfs_dir;

static u8 ec_read(u32 offset)
{
    if (!ec_ram || offset >= EC_MAP_SIZE)
        return 0;
    return readb(ec_ram + offset);
}

static u16 ec_read16(u32 offset)
{
    return ec_read(offset) | (ec_read(offset + 1) << 8);
}

/* ─── debugfs: raw EC RAM dump ─── */

static ssize_t ecram_read(struct file *f, char __user *buf,
                          size_t count, loff_t *ppos)
{
    char tmp[4096];
    int len = 0;
    int start, end;

    if (!ec_ram)
        return -ENODEV;

    if (*ppos >= EC_MAP_SIZE)
        return 0;

    start = *ppos;
    end = min(start + 256, (loff_t)EC_MAP_SIZE);

    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "EC RAM dump (0x%04X - 0x%04X):\n", start, end - 1);
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  addr:  00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F\n");

    for (int base = start; base < end; base += 16) {
        len += scnprintf(tmp + len, sizeof(tmp) - len,
            "  0x%04X:", base);
        for (int off = 0; off < 16; off++) {
            u8 v = ec_read(base + off);
            len += scnprintf(tmp + len, sizeof(tmp) - len,
                " %02X", v);
        }
        len += scnprintf(tmp + len, sizeof(tmp) - len, "\n");
    }

    return simple_read_from_buffer(buf, count, ppos, tmp, len);
}

static const struct file_operations ecram_fops = {
    .owner = THIS_MODULE,
    .read  = ecram_read,
    .llseek = default_llseek,
};

/* ─── debugfs: sensor summary ─── */

static ssize_t sensors_read(struct file *f, char __user *buf,
                            size_t count, loff_t *ppos)
{
    char tmp[2048];
    int len = 0;
    u16 chip_id, fw_ver;
    u8 cpu_t, gpu_t, vrm_t, ic_t, cpu_a, gpu_a;
    u16 fan1, fan2;
    u8 pmode, lockfan;
    u8 fp, fpsize;
    int i;

    if (!ec_ram)
        return -ENODEV;

    chip_id = (ec_read(EC_CHIP_ID1) << 8) | ec_read(EC_CHIP_ID2);
    fw_ver  = ec_read(EC_FW_VER) | (ec_read(EC_FW_VER + 1) << 8);

    cpu_t = ec_read(EC_CPU_TEMP);
    cpu_a = ec_read(EC_CPU_TEMP_ALT);
    gpu_t = ec_read(EC_GPU_TEMP);
    gpu_a = ec_read(EC_GPU_TEMP_ALT);
    vrm_t = ec_read(EC_VRM_TEMP);
    ic_t  = ec_read(EC_IC_TEMP);

    fan1 = ec_read16(EC_FAN1_RPM_LO);
    fan2 = ec_read16(EC_FAN2_RPM_LO);

    pmode   = ec_read(EC_POWERMODE);
    lockfan = ec_read(EC_LOCKFAN);

    fp     = ec_read(EC_FAN_BASE);
    fpsize = ec_read(EC_FAN_BASE + 1);

    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "=== Lenovo Legion EC PoC ===\n\n");
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "Chip ID:   0x%04X  (expected 0x5508 for IT5508)\n", chip_id);
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "FW Ver:    0x%04X\n", fw_ver);
    len += scnprintf(tmp + len, sizeof(tmp) - len, "\n");

    len += scnprintf(tmp + len, sizeof(tmp) - len, "--- Temperatures ---\n");
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  CPU:      %3d°C  (alt: %3d°C)\n", cpu_t, cpu_a);
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  GPU:      %3d°C  (alt: %3d°C)\n", gpu_t, gpu_a);
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  VRM:      %3d°C\n", vrm_t);
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  IC:       %3d°C\n", ic_t);
    len += scnprintf(tmp + len, sizeof(tmp) - len, "\n");

    len += scnprintf(tmp + len, sizeof(tmp) - len, "--- Fans ---\n");
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  Fan1 RPM: %d\n", fan1);
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  Fan2 RPM: %d\n", fan2);
    len += scnprintf(tmp + len, sizeof(tmp) - len, "\n");

    len += scnprintf(tmp + len, sizeof(tmp) - len, "--- Status ---\n");
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  Power mode: 0x%02X (3=perf, 2=bal, 1=quiet, 0xFF=custom)\n", pmode);
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  Lock fan:   0x%02X\n", lockfan);
    len += scnprintf(tmp + len, sizeof(tmp) - len,
        "  Fan curve point: %d/%d\n", fp, fpsize);

    /* Fan curve dump */
    len += scnprintf(tmp + len, sizeof(tmp) - len, "\n--- Fan1 Curve ---\n");
    for (i = 0; i < 10; i++) {
        u8 rpm = ec_read(EC_FAN1_BASE + i);
        len += scnprintf(tmp + len, sizeof(tmp) - len,
            "  pt%d: %d RPM\n", i, rpm * 100);
    }

    /* If all temps are zero, the EC RAM is likely not populated */
    if (cpu_t == 0 && gpu_t == 0 && vrm_t == 0 && ic_t == 0 &&
        cpu_a == 0 && gpu_a == 0 && fan1 == 0 && fan2 == 0) {
        len += scnprintf(tmp + len, sizeof(tmp) - len,
            "\n*** ALL ZERO — Gen 10 (IT5508) likely needs WMI3, not direct EC access ***\n");
    }

    return simple_read_from_buffer(buf, count, ppos, tmp, len);
}

static const struct file_operations sensors_fops = {
    .owner = THIS_MODULE,
    .read  = sensors_read,
    .llseek = default_llseek,
};

/* ─── init / exit ─── */

static int __init legion_ec_init(void)
{
    pr_info("legion_ec: probing EC at phys 0x%08X\n", EC_PHYS_BASE);

    ec_ram = ioremap(EC_PHYS_BASE, EC_MAP_SIZE);
    if (!ec_ram) {
        pr_err("legion_ec: ioremap failed\n");
        return -ENOMEM;
    }
    pr_info("legion_ec: mapped 0x%X bytes to %p\n", EC_MAP_SIZE, ec_ram);

    debugfs_dir = debugfs_create_dir("legion_ec", NULL);
    debugfs_create_file("sensors", 0444, debugfs_dir, NULL, &sensors_fops);
    debugfs_create_file("ecram", 0444, debugfs_dir, NULL, &ecram_fops);

    pr_info("legion_ec: debugfs at /sys/kernel/debug/legion_ec/\n");
    return 0;
}

static void __exit legion_ec_exit(void)
{
    debugfs_remove_recursive(debugfs_dir);
    if (ec_ram)
        iounmap(ec_ram);
    pr_info("legion_ec: unloaded\n");
}

module_init(legion_ec_init);
module_exit(legion_ec_exit);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("PoC");
MODULE_DESCRIPTION("Lenovo Legion EC RAM reader (read-only)");
