savedcmd_legion_hwmon.mod := printf '%s\n'   legion_hwmon.o | awk '!x[$$0]++ { print("./"$$0) }' > legion_hwmon.mod
