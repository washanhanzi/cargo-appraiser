import ky from 'ky'
import { workspace, ExtensionContext, window, Uri, StatusBarAlignment } from 'vscode'
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind
} from 'vscode-languageclient/node'


let client: LanguageClient

export async function activate(context: ExtensionContext) {
    // Get the binary path
    let serverPath = await languageServerBinaryPath(context)

    const traceOutputChannel = window.createOutputChannel("Cargo-appraiser Langauage Server")

    if (!serverPath) {
        window.showErrorMessage('CARGO_APPRAISER_PATH environment variable is not set. Unable to start the language server.')
        return
    }

    // If the extension is launched in debug mode then the debug server options are used
    // Otherwise the run options are used
    // add env rust log level set to debug
    const serverOptions: ServerOptions = {
        run: {
            command: serverPath,
            args: ["--renderer", "vscode", "--client-capabilities", "readFile"],
            transport: TransportKind.stdio,
        },
        debug: {
            command: serverPath,
            args: ["--renderer", "vscode", "--client-capabilities", "readFile"],
            transport: TransportKind.stdio,
        }
    }

    // Options to control the language client
    const clientOptions: LanguageClientOptions = {
        // Register the server for TOML documents
        documentSelector: [{ scheme: 'file', language: 'toml' }],
        synchronize: {
            // Notify the server about file changes to '.clientrc files contained in the workspace
            fileEvents: workspace.createFileSystemWatcher('**/Cargo.lock')
        },
        outputChannel: traceOutputChannel,
    }

    // Create the language client and start the client.
    client = new LanguageClient(
        'cargoAppraiser',
        'LSP server for Cargo.toml',
        serverOptions,
        clientOptions
    )

    client.onRequest("textDocument/readFile", async (params, next) => {
        const uri = Uri.parse(params.uri)
        const document = await workspace.openTextDocument(uri)
        return {
            content: document.getText()
        }
    })

    // Start the client. This will also launch the server
    client.start()
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined
    }
    return client.stop()
}

// Function to download and get the path of the language server binary
async function languageServerBinaryPath(context: ExtensionContext): Promise<string> {
    //check env path
    const serverPath = process.env.CARGO_APPRAISER_PATH

    if (serverPath) {
        return serverPath
    }

    //TODO get path from config

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