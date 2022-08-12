#! /bin/sh

grep -q '^sdio_wifi_pwr\b' /proc/modules && exit 1

SCRIPTS_DIR=$(dirname "$0")
PRE_UP_SCRIPT=$SCRIPTS_DIR/wifi-pre-up.sh
[ -e "$PRE_UP_SCRIPT" ] && $PRE_UP_SCRIPT

insmod /drivers/"$PLATFORM"/wifi/sdio_wifi_pwr.ko

if [ -e /drivers/"${PLATFORM}/${WIFI_MODULE}".ko ]; then
	insmod /drivers/"${PLATFORM}/${WIFI_MODULE}".ko
else
	insmod /drivers/"$PLATFORM"/wifi/"$WIFI_MODULE".ko
fi

REM_TRIES=20
while [ "$REM_TRIES" -gt 0 ] ; do
	[ -e /sys/class/net/"$INTERFACE" ] && break
	REM_TRIES=$((REM_TRIES-1))
	sleep 0.2
done

ifconfig "$INTERFACE" up
[ "$WIFI_MODULE" = dhd ] && wlarm_le -i "$INTERFACE" up

pidof wpa_supplicant > /dev/null || wpa_supplicant -D wext -s -i "$INTERFACE" -c /etc/wpa_supplicant/wpa_supplicant.conf -C /var/run/wpa_supplicant -B

udhcpc -S -i "$INTERFACE" -s /etc/udhcpc.d/default.script -t15 -T10 -A3 -b -q > /dev/null &

POST_UP_SCRIPT=$SCRIPTS_DIR/wifi-post-up.sh
[ -e "$POST_UP_SCRIPT" ] && $POST_UP_SCRIPT
