#! /bin/sh

# TODO bootstrapping, replace with own zip file
version=$(cargo pkgid | cut -d '#' -f 2)
archive="plato-0.9.30.zip"
if ! [ -e "$archive" ]; then
    info_url="https://api.github.com/repos/baskerville/plato/releases/tags/${version}"
    echo "Downloading ${archive}."
    release_url=$(wget -q -O - "$info_url" | jq -r ".assets[] | select(.name == \"$archive\").browser_download_url")
    wget -q --show-progress "$release_url"
fi
unzip "$archive" "$@"
