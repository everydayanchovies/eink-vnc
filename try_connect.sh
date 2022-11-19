#!/usr/bin/env bash

eink_ip_addr="192.168.1.146"
eink_user="root"
vnc_ip_addr="192.168.1.195"
vnc_port="5902"
vnc_password=$1
vnc_control_password=$2
user="max"

eink_resolution="1264x1680"

has_connected=false

cleanup() {
    echo "Cleaning up..."
    echo 100 | sudo tee /sys/class/backlight/radeon_bl0/brightness
    vncserver -kill :2
    sudo pkill vncviewer
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
    sudo -u $user ssh root@$eink_ip_addr "pkill -f einkvnc"
    sudo -u $user ssh root@$eink_ip_addr "fbink -k && nohup /mnt/onboard/.adds/einkvnc ${vnc_ip_addr} ${vnc_port} --password '${vnc_password}' --contrast 2 --graypoint 200"
}

cleanup

while :; do
	echo 'loop';

    if [ -e /home/max/projects/eink-emacs/suspend_flag]; then
	    rm /home/max/projects/eink-emacs/suspend_flag;
	    has_connected=false;
    fi

    if nc -z $eink_ip_addr 22 2>/dev/null; then
	    echo 'found ereader on network';
        if ! $has_connected; then
            has_connected=true
            echo "eInk device detected!"

            echo 0 | sudo tee /sys/class/backlight/radeon_bl0/brightness
            sudo loginctl unlock-sessions

            sleep 1

            serve_vnc
            sleep 1
            connect &
        fi
    else
        if $has_connected; then
            sudo loginctl lock-sessions
            sleep 1
            cleanup
            has_connected=false
        fi
    fi
    sleep 1
done
