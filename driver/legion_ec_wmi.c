/*
 * legion_ec_wmi.c — PoC: call EC ACPI methods for fan speed + probe temp methods.
 *
 * Build: make
 * Test:  sudo insmod legion_ec_wmi.ko && cat /sys/kernel/debug/legion_ec/sensors
 *
 * Does NOT register as a WMI driver — uses ACPI handle directly.
 * If FANS/FA2S return values, we've found the EC. Then we probe
 * for temperature methods (CTMP, GTMP, etc.) and report results.
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/moduleparam.h>
#include <linux/debugfs.h>
#include <linux/acpi.h>

static acpi_handle ec_handle;
static struct dentry *debugfs_dir;

/* Known ACPI paths for Lenovo Legion EC — try each until one works */
static const char *ec_paths[] = {
    "\\_SB.PCI0.LPC0.EC0",
    "\\_SB.PCI0.SBRG.EC0",
    "\\_SB.PCI0.LPCB.EC0",
    "\\_SB.PC00.LPC0.EC0",
    "\\_SB.PCI0.LPC0.EC",
    NULL
};

static int ec_eval(const char *method, unsigned long long *val)
{
    if (!ec_handle)
        return -ENODEV;
    if (ACPI_FAILURE(acpi_evaluate_integer(ec_handle, (char *)method, NULL, val)))
        return -EIO;
    return 0;
}

static ssize_t sensors_read(struct file *f, char __user *buf,
                            size_t count, loff_t *ppos)
{
    char *tmp;
    int len = 0;
    unsigned long long v;

    tmp = kmalloc(8192, GFP_KERNEL);
    if (!tmp) return -ENOMEM;

    if (!ec_handle) {
        len += scnprintf(tmp + len, 8192 - len,
            "EC handle not found.\n"
            "Tried paths:\n");
        for (int i = 0; ec_paths[i]; i++)
            len += scnprintf(tmp + len, 8192 - len,
                "  %s\n", ec_paths[i]);
        goto done;
    }

    len += scnprintf(tmp + len, 8192 - len,
        "=== Legion ACPI EC PoC ===\n\n");

    /* ─── Fan speeds ─── */
    len += scnprintf(tmp + len, 8192 - len, "--- Fan Speeds ---\n");
    if (ec_eval("FANS", &v) == 0)
        len += scnprintf(tmp + len, 8192 - len, "  Fan1: %llu RPM\n", v);
    else
        len += scnprintf(tmp + len, 8192 - len, "  Fan1: N/A\n");

    if (ec_eval("FA2S", &v) == 0)
        len += scnprintf(tmp + len, 8192 - len, "  Fan2: %llu RPM\n", v);
    else
        len += scnprintf(tmp + len, 8192 - len, "  Fan2: N/A\n");

    if (ec_eval("FA3S", &v) == 0)
        len += scnprintf(tmp + len, 8192 - len, "  Fan3: %llu RPM\n", v);
    else
        len += scnprintf(tmp + len, 8192 - len, "  Fan3: N/A\n");

    if (ec_eval("FA4S", &v) == 0)
        len += scnprintf(tmp + len, 8192 - len, "  Fan4: %llu RPM\n", v);
    else
        len += scnprintf(tmp + len, 8192 - len, "  Fan4: N/A\n");

    /* ─── Probe temperature methods ─── */
    len += scnprintf(tmp + len, 8192 - len, "\n--- Temperature Probe ---\n");
    const char *methods[] = {
        "TMP1", "TMP2", "TMP3", "CTMP", "GTMP", "RTMP",
        "ETMP", "FTMP", "TCPU", "TGPU", "TEMP", "TSEN",
        "TCRT", "TPSV", "TAC0", "TAC1", "TAC2", "TAC3",
        NULL
    };
    int found = 0;
    for (int i = 0; methods[i]; i++) {
        if (ec_eval(methods[i], &v) == 0) {
            if (v > 2000)  /* probably decikelvin — convert integer */
                len += scnprintf(tmp + len, 8192 - len,
                    "  %s: %llu  (~%lld C)\n",
                    methods[i], v, ((long long)v - 2732) / 10);
            else
                len += scnprintf(tmp + len, 8192 - len,
                    "  %s: %llu C\n", methods[i], v);
            found++;
        }
    }
    if (!found)
        len += scnprintf(tmp + len, 8192 - len,
            "  No standard temp methods found.\n"
            "  Temperatures likely require full WMI3 protocol.\n");

    /* ─── Power mode ─── */
    len += scnprintf(tmp + len, 8192 - len, "\n--- Power Mode ---\n");
    if (ec_eval("SPMO", &v) == 0)
        len += scnprintf(tmp + len, 8192 - len,
            "  SPMO: %llu (0=quiet,1=balanced,2=perf,3=custom)\n", v);
    else if (ec_eval("GPMD", &v) == 0)
        len += scnprintf(tmp + len, 8192 - len,
            "  GPMD: %llu\n", v);
    else
        len += scnprintf(tmp + len, 8192 - len,
            "  No power mode method found.\n");

    len += scnprintf(tmp + len, 8192 - len, "\n");
    len += scnprintf(tmp + len, 8192 - len,
        "CPU Tctl (k10temp): check with:\n");
    len += scnprintf(tmp + len, 8192 - len,
        "  cat /sys/class/hwmon/hwmon5/temp1_input\n");

done:
    {
        ssize_t ret = simple_read_from_buffer(buf, count, ppos, tmp, len);
        kfree(tmp);
        return ret;
    }
}

static const struct file_operations sensors_fops = {
    .owner = THIS_MODULE,
    .read  = sensors_read,
    .llseek = default_llseek,
};

static int __init legion_ec_wmi_init(void)
{
    pr_info("legion_ec_wmi: probing EC ACPI paths...\n");

    for (int i = 0; ec_paths[i]; i++) {
        acpi_status st = acpi_get_handle(NULL, ec_paths[i], &ec_handle);
        if (ACPI_SUCCESS(st)) {
            pr_info("legion_ec_wmi: EC found at %s\n", ec_paths[i]);
            break;
        }
    }

    if (!ec_handle) {
        pr_err("legion_ec_wmi: EC handle not found.\n");
        pr_err("legion_ec_wmi: Run: acpidump | grep EC0  to find ACPI path\n");
    }

    debugfs_dir = debugfs_create_dir("legion_ec", NULL);
    debugfs_create_file("sensors", 0444, debugfs_dir, NULL, &sensors_fops);

    return 0;
}

static void __exit legion_ec_wmi_exit(void)
{
    debugfs_remove_recursive(debugfs_dir);
    pr_info("legion_ec_wmi: unloaded\n");
}

module_init(legion_ec_wmi_init);
module_exit(legion_ec_wmi_exit);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("PoC");
MODULE_DESCRIPTION("Lenovo Legion ACPI EC sensor probe");
