import { workspace, ExtensionContext, window, Uri } from 'vscode'
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind
} from 'vscode-languageclient/node'
import { DecorationCtrl } from './decoration'
import { config } from './config'
import { languageServerBinaryPath } from './serverPath'

let client: LanguageClient

// Helper function to prepare environment variables for the server
function prepareEnv(configOptions?: any): { [key: string]: string | undefined } {
    // Start with all current environment variables
    const currentEnv = { ...process.env }

    // Add any user-defined environment variables from configuration
    const extraEnv = configOptions?.extraEnv || {}
    if (extraEnv && typeof extraEnv === 'object') {
        console.log('Adding user-defined environment variables:', Object.keys(extraEnv))
        Object.assign(currentEnv, extraEnv)
    }

    return currentEnv
}

export async function activate(context: ExtensionContext) {
    config.init()

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
            options: {
                env: prepareEnv(config.getInitializationOptions())
            }
        },
        debug: {
            command: serverPath,
            args: ["--renderer", "vscode", "--client-capabilities", "readFile"],
            transport: TransportKind.stdio,
            options: {
                env: prepareEnv(config.getInitializationOptions())
            }
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
        initializationOptions: config.getInitializationOptions(),
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

    const decorationCtrl = new DecorationCtrl()
    decorationCtrl.listen(client)

    workspace.onDidChangeConfiguration(config.onChange, config)

    context.subscriptions.push(
        window.onDidChangeActiveColorTheme(config.onThemeChange),
        window.onDidChangeActiveTextEditor(decorationCtrl.onDidChangeActiveTextEditor, decorationCtrl)
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
