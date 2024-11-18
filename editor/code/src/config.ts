import { ColorThemeKind, ConfigurationChangeEvent, window, workspace } from "vscode"

type DecorationColor = {
    light: DecorationColorItem
    dark: DecorationColorItem
    highContrast: DecorationColorItem
    highContrastLight: DecorationColorItem
}

type DecorationColorItem = {
    notParsed: string
    latest: string
    local: string
    notInstalled: string
    mixedUpgradeable: string
    compatibleLatest: string
    nonCompatibleLatest: string
    yanked: string
    git: string
}

const defaultLight: DecorationColorItem = {
    notParsed: "#808080",
    latest: "#006400",
    local: "#0000FF",
    notInstalled: "#808080",
    mixedUpgradeable: "#FF8C00",
    compatibleLatest: "#FF8C00",
    nonCompatibleLatest: "#FF8C00",
    yanked: "#FF0000",
    git: "#800080"
}

const defaultDark: DecorationColorItem = {
    notParsed: "#808080",
    latest: "#006400",
    local: "#0000FF",
    notInstalled: "#808080",
    mixedUpgradeable: "#FF8C00",
    compatibleLatest: "#FF8C00",
    nonCompatibleLatest: "#FF8C00",
    yanked: "#FF0000",
    git: "#800080"
}

type InitializationOptions = {
    decorationFormatter: {
        latest: string
        local: string
        not_installed: string
        waiting: string
        mixed_upgradeable: string
        compatible_latest: string
        noncompatible_latest: string
        yanked: string
        git: string
    }
}

class Config {
    private currentTheme: ColorThemeKind
    private colors: DecorationColor
    private initializationOptions: InitializationOptions | null = null

    constructor() {
        this.currentTheme = window.activeColorTheme.kind
        this.colors = {
            light: defaultLight,
            dark: defaultDark,
            highContrast: defaultDark,
            highContrastLight: defaultLight
        }
        this.updateColor()

    }

    init() {
        const formatter = workspace.getConfiguration("cargo-appraiser").get("decorationFormatter")
        if (typeof formatter === "object") {
            this.initializationOptions = { decorationFormatter: formatter as any }
        }
    }

    getInitializationOptions() {
        return this.initializationOptions
    }

    getCurrentTheme() {
        return this.currentTheme
    }

    getColor(kind: string): string {
        switch (this.currentTheme) {
            case ColorThemeKind.Light:
                return this.colors.light[kind]
            case ColorThemeKind.Dark:
                return this.colors.dark[kind]
            case ColorThemeKind.HighContrast:
                return this.colors.highContrast[kind]
            case ColorThemeKind.HighContrastLight:
                return this.colors.highContrastLight[kind]
        }
    }

    onThemeChange(theme) {
        this.currentTheme = theme.kind
    }


    onChange(e: ConfigurationChangeEvent) {
        if (e.affectsConfiguration("cargo-appraiser.decorationColor")) {
            this.updateColor()
        }
    }

    updateInitializationOptions() {
    }

    updateColor() {
        const notParsed = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.notParsed")
        if (typeof notParsed === "string") {
            this.colors.light.notParsed = notParsed
        }
        const latest = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.latest")
        if (typeof latest === "string") {
            this.colors.light.latest = latest
        }
        const local = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.local")
        if (typeof local === "string") {
            this.colors.light.local = local
        }
        const notInstalled = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.notInstalled")
        if (typeof notInstalled === "string") {
            this.colors.light.notInstalled = notInstalled
        }
        const mixedUpgradeable = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.mixedUpgradeable")
        if (typeof mixedUpgradeable === "string") {
            this.colors.light.mixedUpgradeable = mixedUpgradeable
        }
        const compatibleLatest = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.compatibleLatest")
        if (typeof compatibleLatest === "string") {
            this.colors.light.compatibleLatest = compatibleLatest
        }
        const nonCompatibleLatest = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.nonCompatibleLatest")
        if (typeof nonCompatibleLatest === "string") {
            this.colors.light.nonCompatibleLatest = nonCompatibleLatest
        }
        const yanked = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.yanked")
        if (typeof yanked === "string") {
            this.colors.light.yanked = yanked
        }
        const git = workspace.getConfiguration("cargo-appraiser").get("decorationColor.light.git")
        if (typeof git === "string") {
            this.colors.light.git = git
        }

        const notParsedDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.notParsed")
        if (typeof notParsedDark === "string") {
            this.colors.dark.notParsed = notParsedDark
        }
        const latestDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.latest")
        if (typeof latestDark === "string") {
            this.colors.dark.latest = latestDark
        }
        const localDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.local")
        if (typeof localDark === "string") {
            this.colors.dark.local = localDark
        }
        const notInstalledDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.notInstalled")
        if (typeof notInstalledDark === "string") {
            this.colors.dark.notInstalled = notInstalledDark
        }
        const mixedUpgradeableDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.mixedUpgradeable")
        if (typeof mixedUpgradeableDark === "string") {
            this.colors.dark.mixedUpgradeable = mixedUpgradeableDark
        }
        const compatibleLatestDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.compatibleLatest")
        if (typeof compatibleLatestDark === "string") {
            this.colors.dark.compatibleLatest = compatibleLatestDark
        }
        const nonCompatibleLatestDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.nonCompatibleLatest")
        if (typeof nonCompatibleLatestDark === "string") {
            this.colors.dark.nonCompatibleLatest = nonCompatibleLatestDark
        }
        const yankedDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.yanked")
        if (typeof yankedDark === "string") {
            this.colors.dark.yanked = yankedDark
        }
        const gitDark = workspace.getConfiguration("cargo-appraiser").get("decorationColor.dark.git")
        if (typeof gitDark === "string") {
            this.colors.dark.git = gitDark
        }

        const notParsedHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.notParsed")
        if (typeof notParsedHighContrast === "string") {
            this.colors.highContrast.notParsed = notParsedHighContrast
        }
        const latestHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.latest")
        if (typeof latestHighContrast === "string") {
            this.colors.highContrast.latest = latestHighContrast
        }
        const localHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.local")
        if (typeof localHighContrast === "string") {
            this.colors.highContrast.local = localHighContrast
        }
        const notInstalledHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.notInstalled")
        if (typeof notInstalledHighContrast === "string") {
            this.colors.highContrast.notInstalled = notInstalledHighContrast
        }
        const mixedUpgradeableHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.mixedUpgradeable")
        if (typeof mixedUpgradeableHighContrast === "string") {
            this.colors.highContrast.mixedUpgradeable = mixedUpgradeableHighContrast
        }
        const compatibleLatestHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.compatibleLatest")
        if (typeof compatibleLatestHighContrast === "string") {
            this.colors.highContrast.compatibleLatest = compatibleLatestHighContrast
        }
        const nonCompatibleLatestHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.nonCompatibleLatest")
        if (typeof nonCompatibleLatestHighContrast === "string") {
            this.colors.highContrast.nonCompatibleLatest = nonCompatibleLatestHighContrast
        }
        const yankedHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.yanked")
        if (typeof yankedHighContrast === "string") {
            this.colors.highContrast.yanked = yankedHighContrast
        }
        const gitHighContrast = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrast.git")
        if (typeof gitHighContrast === "string") {
            this.colors.highContrast.git = gitHighContrast
        }

        const notParsedHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.notParsed")
        if (typeof notParsedHighContrastLight === "string") {
            this.colors.highContrastLight.notParsed = notParsedHighContrastLight
        }
        const latestHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.latest")
        if (typeof latestHighContrastLight === "string") {
            this.colors.highContrastLight.latest = latestHighContrastLight
        }
        const localHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.local")
        if (typeof localHighContrastLight === "string") {
            this.colors.highContrastLight.local = localHighContrastLight
        }
        const notInstalledHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.notInstalled")
        if (typeof notInstalledHighContrastLight === "string") {
            this.colors.highContrastLight.notInstalled = notInstalledHighContrastLight
        }
        const mixedUpgradeableHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.mixedUpgradeable")
        if (typeof mixedUpgradeableHighContrastLight === "string") {
            this.colors.highContrastLight.mixedUpgradeable = mixedUpgradeableHighContrastLight
        }
        const compatibleLatestHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.compatibleLatest")
        if (typeof compatibleLatestHighContrastLight === "string") {
            this.colors.highContrastLight.compatibleLatest = compatibleLatestHighContrastLight
        }
        const nonCompatibleLatestHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.nonCompatibleLatest")
        if (typeof nonCompatibleLatestHighContrastLight === "string") {
            this.colors.highContrastLight.nonCompatibleLatest = nonCompatibleLatestHighContrastLight
        }
        const yankedHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.yanked")
        if (typeof yankedHighContrastLight === "string") {
            this.colors.highContrastLight.yanked = yankedHighContrastLight
        }
        const gitHighContrastLight = workspace.getConfiguration("cargo-appraiser").get("decorationColor.highContrastLight.git")
        if (typeof gitHighContrastLight === "string") {
            this.colors.highContrastLight.git = gitHighContrastLight
        }
    }
}

export const config = new Config()
