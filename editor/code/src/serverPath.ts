import ky from 'ky'
import { workspace, ExtensionContext, window, Uri, StatusBarAlignment } from 'vscode'
import { config } from './config'

// Function to download and get the path of the language server binary
export async function languageServerBinaryPath(context: ExtensionContext): Promise<string> {
    // First, check if user has configured a custom server path in settings
    const configuredPath = config.getInitializationOptions()?.serverPath
    if (configuredPath) {
        console.log(`Using configured server path: ${configuredPath}`)
        return configuredPath
    }

    // Next, check env path
    const serverPath = process.env.CARGO_APPRAISER_PATH
    if (serverPath) {
        console.log(`Using environment variable server path: ${serverPath}`)
        return serverPath
    }

    console.log('No server path configured. Attempting to download from GitHub...')

    //check if file system is writable
    if (!workspace.fs.isWritableFileSystem("file")) {
        throw new Error("File system is not writable")
    }

    const fs = require('fs')
    const { promisify } = require('util')
    const chmod = promisify(fs.chmod)

    // Fetch latest release info
    const releaseInfo = await ky.get('https://api.github.com/repos/washanhanzi/cargo-appraiser/releases/latest', {
        headers: { 'User-Agent': 'VSCode-Extension' }
    }).json() as any

    // Determine platform and architecture
    const platform = process.platform
    const arch = process.arch

    const assetName = `cargo-appraiser-${platform === 'win32' ? 'windows' : platform === 'darwin' ? 'darwin' : 'linux'}-${arch === 'arm64' ? 'arm64' : 'amd64'}${platform === 'win32' ? '.exe' : ''}`

    const asset = releaseInfo.assets.find((asset: any) => asset.name === assetName)
    if (!asset) {
        throw new Error(`No asset found matching ${assetName}`)
    }

    const versionDir = `cargo-appraiser-${releaseInfo.tag_name}`
    const binaryName = platform === 'win32' ? 'cargo-appraiser.exe' : 'cargo-appraiser'

    const uri = Uri.joinPath(context.globalStorageUri, versionDir)
    const binaryPath = Uri.joinPath(uri, binaryName)
    await workspace.fs.createDirectory(uri)

    const content = await workspace.fs.readDirectory(uri)

    //directory is not empty
    if (content.length !== 0) {
        return binaryPath.fsPath
    }

    //show status bar message with version info
    const statusBarItem = window.createStatusBarItem("download-lsp-server", StatusBarAlignment.Left, 0)

    //claude write this
    // let spinnerIndex = 0
    // const spinnerChars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏']
    // const spinnerInterval = setInterval(() => {
    //     statusBarItem.text = `${spinnerChars[spinnerIndex]} Downloading Cargo-appraiser LSP server ${releaseInfo.tag_name}...`
    //     spinnerIndex = (spinnerIndex + 1) % spinnerChars.length
    // }, 100)

    statusBarItem.text = `$(loading~spin) Downloading cargo-appraiser server ${releaseInfo.tag_name}...`
    statusBarItem.show()

    const response = await ky.get(asset.browser_download_url)
    const buffer = await response.arrayBuffer()
    await workspace.fs.writeFile(binaryPath, new Uint8Array(buffer))

    await chmod(binaryPath.fsPath, '755')
    statusBarItem.dispose()

    return binaryPath.fsPath
}
