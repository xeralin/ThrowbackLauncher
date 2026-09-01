<p>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/night.svg">
    <img alt="Throwback Launcher" src=".github/day.svg" height="42">
  </picture> <br>
  It downloads, manages and launches any older season of Rainbow Six Siege. Not affiliated with, endorsed by, or sponsored by Ubisoft.
</p>

**Windows**

1. Download `Installer.exe` from the [latest release](https://github.com/xeralin/ThrowbackLauncher/releases/latest) and run it
2. If Windows Security removes it, allow it under **Protection history** and run it again
3. If SmartScreen shows **Windows protected your PC**, click **More info** and then **Run anyway**
4. Press **Install** and wait until the installation is done

**Linux**

1. Download `ThrowbackLauncher.AppImage` from the [latest release](https://github.com/xeralin/ThrowbackLauncher/releases/latest)
2. Enable **Allow executing file as program** in the file properties to make it executable
3. Open it

**Building**

For a local run on Linux you need [pnpm](https://pnpm.io/) and Python 3.14 or newer.

```sh
git clone https://github.com/xeralin/ThrowbackLauncher.git
cd ThrowbackLauncher
pnpm -C next install
python -m venv .venv && .venv/bin/pip install -r app/requirements.txt
./run.sh
```

<p>
  <a href="https://github.com/xeralin/ThrowbackLauncher/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/xeralin/ThrowbackLauncher?style=flat&color=c0152a" /></a>
  <a href="https://github.com/xeralin/ThrowbackLauncher/releases"><img alt="downloads" src="https://img.shields.io/github/downloads/xeralin/ThrowbackLauncher/total?style=flat&color=c0152a" /></a>
  <a href="https://discord.gg/r6s-operation-throwback-2-0-1092820800203141130"><img alt="discord" src="https://img.shields.io/discord/1092820800203141130?style=flat&label=discord&color=e8e0d5" /></a>
</p>
