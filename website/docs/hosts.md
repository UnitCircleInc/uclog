# The hosts

Every host reads the same device, the same logdata and the same wire protocol; pick the one that
fits where you work. These screenshots are made by the hosts' own tests, against the simulated
device (`build.py screenshots`).

## Web app

[uclog.unitcircle.ca/app/](web-app.md): logs, a terminal, commands, plots, device information and
firmware updates, in Chrome or Edge.

![The web app's logs](assets/screens/web-logs-light.png#only-light)
![The web app's logs](assets/screens/web-logs-dark.png#only-dark)

![Plots in the web app](assets/screens/web-plots-light.png)

## VS Code

The same app in a VS Code panel: the logdata comes from the workspace's build, and clicking a log
opens the source line that wrote it.

![ucLog in VS Code](assets/screens/vscode-logs.png)

## `uclog gui`

A native window (egui), for when there is no browser or VS Code.

![uclog gui](assets/screens/gui-logs-light.png#only-light)
![uclog gui](assets/screens/gui-logs-dark.png#only-dark)

![Plots in uclog gui](assets/screens/gui-plots-light.png)

## `uclog tui`

The terminal UI, also over SSH.

![uclog tui](assets/screens/tui-logs.svg)
