#!/usr/bin/env bash

eink_ip_addr_usb="192.168.2.2"
eink_ip_addr_wifi="192.168.1.146"
eink_user="root"
vnc_ip_addr_usb="192.168.2.1"
vnc_ip_addr_wifi="192.168.1.195"
vnc_port="5902"
vnc_password=$1
vnc_control_password=$2
user="max"

eink_resolution="1264x1680"

has_connected_wifi=false
has_connected_usb=false

cleanup() {
    echo "Cleaning up..."
    echo 100 | sudo tee /sys/class/backlight/radeon_bl0/brightness
    vncserver -kill :2
    sudo pkill vncviewer
    has_connected_wifi=false
    has_connected_usb=false
}

trap cleanup EXIT

serve_vnc() {
    echo "Serving..."
    sudo -u $user vncserver :2 -geometry $eink_resolution -compatiblekbd &

    sleep 1

    sudo pkill vncviewer

    echo $vnc_control_password | sudo -u $user vncviewer -autopass -bgr233 -compresslevel 5 -nocursorshape localhost:$vnc_port &

    sleep 3

    sudo wmctrl -a TightVNC
}

connect() {
    echo "Connecting..."
    echo $eink_ip_addr
    echo $vnc_ip_addr

    sudo -u $user ssh root@$eink_ip_addr "pkill -f einkvnc"
    sudo -u $user ssh root@$eink_ip_addr "fbink -k && nohup /mnt/onboard/.adds/einkvnc ${vnc_ip_addr} ${vnc_port} --password '${vnc_password}' --contrast 1.5 --graypoint 150"
}

cleanup

while :; do

    if [ -e /home/max/projects/eink-emacs/suspend_flag ]; then
	   rm /home/max/projects/eink-emacs/suspend_flag;
	   has_connected_usb=false;
	   has_connected_wifi=false;
    fi

    if ! $has_connected_wifi; then

    sudo ifconfig usb0 192.168.2.1 >/dev/null 2>/dev/null || sudo ifconfig enxee4900038831 192.168.2.1 >/dev/null 2>/dev/null
    if [ $? -eq 0 ]; then
        if ! $has_connected_usb; then
            eink_ip_addr=$eink_ip_addr_usb
            vnc_ip_addr=$vnc_ip_addr_usb

            has_connected_usb=true
            echo "eInk device detected via usb!"

            echo 0 | sudo tee /sys/class/backlight/radeon_bl0/brightness
            sudo loginctl unlock-sessions

            sleep 1

            serve_vnc
            sleep 1
            connect &
        fi
    else
        if $has_connected_usb; then
            sudo loginctl lock-sessions
            sudo systemctl suspend
            sleep 1
            cleanup
            has_connected_usb=false
        fi
    fi

    fi

    if ! $has_connected_usb; then

    if nc -z $eink_ip_addr_wifi 22 2>/dev/null; then
        if ! $has_connected_wifi; then
            eink_ip_addr=$eink_ip_addr_wifi
            vnc_ip_addr=$vnc_ip_addr_wifi

            has_connected_wifi=true
            echo "eInk device detected on wifi!"

            echo 0 | sudo tee /sys/class/backlight/radeon_bl0/brightness
            sudo loginctl unlock-sessions

            sleep 1

            serve_vnc
            sleep 1
            connect &
        fi
    else
        if $has_connected_wifi; then
            sudo loginctl lock-sessions
            sleep 1
            cleanup
            has_connected_wifi=false
        fi
    fi

    fi


    sleep 2
done

