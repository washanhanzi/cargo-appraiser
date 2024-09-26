import { workspace, ExtensionContext, window } from 'vscode'
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind
} from 'vscode-languageclient/node'

let client: LanguageClient

export function activate(context: ExtensionContext) {

    const traceOutputChannel = window.createOutputChannel("Cargo-appraiser Langauage Server")
    // Read the server path from the environment variable
    const serverPath = process.env.CARGO_APPRAISER_PATH

    if (!serverPath) {
        window.showErrorMessage('CARGO_APPRAISER_PATH environment variable is not set. Unable to start the language server.')
        return
    }

    // If the extension is launched in debug mode then the debug server options are used
    // Otherwise the run options are used
    const serverOptions: ServerOptions = {
        run: {
            command: serverPath,
            args: ["--renderer", "vscode"],
            transport: TransportKind.stdio,
        },
        debug: {
            command: serverPath,
            args: ["--renderer", "vscode"],
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
        outputChannel: traceOutputChannel
    }

    // Create the language client and start the client.
    client = new LanguageClient(
        'cargoAppraiser',
        'LSP server for Cargo.toml',
        serverOptions,
        clientOptions
    )

    // Start the client. This will also launch the server
    client.start()
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined
    }
    return client.stop()
}