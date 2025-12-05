import { Range, TextEditor, TextEditorDecorationType, Uri, window } from "vscode"
import { LanguageClient } from "vscode-languageclient/node"
import { config } from "./config"

//decoration type
type Decoration = {
    text: string
    color: string
    margin: string
    range: Range
    handle: TextEditorDecorationType
}

type DecorationData = {
    id: string
    text: string
    kind: string
    range: Range
}

export class DecorationCtrl {
    map: Map<string, Map<string, Decoration>> = new Map()

    listen(client: LanguageClient) {
        // Old API (single operations) - for backward compatibility
        client.onRequest("textDocument/decoration/create", async (params: { uri: string, id: string, text: string, range: Range, kind: string }) => {
            this.create(Uri.parse(params.uri), params.id, params.text, params.range, params.kind)
            return
        })

        client.onRequest("textDocument/decoration/updateRange", async (params: { uri: string, id: string, range: Range }) => {
            this.updateRange(Uri.parse(params.uri), params.id, params.range)
            return
        })

        client.onRequest("textDocument/decoration/delete", async (params: { uri: string, id: string }) => {
            this.delete(Uri.parse(params.uri), params.id)
            return
        })

        client.onRequest("textDocument/decoration/reset", async (params: { uri: string }) => {
            this.reset(Uri.parse(params.uri))
            return
        })

        // New API (batch operations)
        client.onRequest("textDocument/decoration/replaceAll", async (params: { uri: string, decorations: DecorationData[] }) => {
            this.replaceAll(Uri.parse(params.uri), params.decorations)
            return
        })
    }

    onDidChangeActiveTextEditor(editor: TextEditor | undefined) {
        if (!editor) {
            return
        }
        const path = editor.document.uri.path
        const innerMap = this.map.get(path)
        if (!innerMap) {
            return
        }
        innerMap.forEach((deco) => {
            editor.setDecorations(deco.handle, [deco.range])
        })
    }

    // Old API methods (single operations)
    create(uri: Uri, id: string, text: string, range: Range, kind: string) {
        const color = config.getColor(kind)

        const deco: Decoration = {
            text,
            color,
            margin: '0 0 0 4em',
            range,
            handle: window.createTextEditorDecorationType({
                after: {
                    contentText: text,
                    color,
                    margin: '0 0 0 4em'
                }
            })
        }
        // Get or create the inner map for this URI
        let innerMap = this.map.get(uri.path)
        if (!innerMap) {
            innerMap = new Map<string, Decoration>()
            this.map.set(uri.path, innerMap)
        } else {
            const d = innerMap.get(id)
            if (d) {
                d.handle.dispose()
            }
        }

        // Store and apply the new decoration
        innerMap.set(id, deco)

        const editor = window.activeTextEditor
        if (!editor) {
            return
        }
        if (editor.document.uri.path !== uri.path) {
            return
        }
        editor.setDecorations(deco.handle, [range])
    }

    updateRange(uri: Uri, id: string, range: Range) {
        const innerMap = this.map.get(uri.path)
        if (!innerMap) {
            return
        }
        let deco = innerMap.get(id)
        if (!deco) {
            return
        }
        deco.range = range
        innerMap.set(id, deco)

        const editor = window.activeTextEditor
        if (!editor) {
            return
        }
        if (editor.document.uri.path !== uri.path) {
            return
        }
        editor.setDecorations(deco.handle, [range])
    }

    delete(uri: Uri, id: string) {
        const innerMap = this.map.get(uri.path)
        if (!innerMap) {
            return
        }
        const d = innerMap.get(id)
        if (d) {
            d.handle.dispose()
            innerMap.delete(id)
        }
    }

    reset(uri: Uri) {
        const innerMap = this.map.get(uri.path)
        if (!innerMap) {
            return
        }
        innerMap.forEach((deco) => {
            deco.handle.dispose()
        })
        this.map.delete(uri.path)
    }

    // New API methods (batch operations)
    replaceAll(uri: Uri, decorations: DecorationData[]) {
        // First, dispose all existing decorations for this URI
        const existingMap = this.map.get(uri.path)
        if (existingMap) {
            existingMap.forEach((deco) => {
                deco.handle.dispose()
            })
        }

        // Create new map for decorations
        const innerMap = new Map<string, Decoration>()
        this.map.set(uri.path, innerMap)

        const editor = window.activeTextEditor
        const isActiveEditor = editor && editor.document.uri.path === uri.path

        // Create all new decorations
        for (const data of decorations) {
            const color = config.getColor(data.kind)
            const deco: Decoration = {
                text: data.text,
                color,
                margin: '0 0 0 4em',
                range: data.range,
                handle: window.createTextEditorDecorationType({
                    after: {
                        contentText: data.text,
                        color,
                        margin: '0 0 0 4em'
                    }
                })
            }
            innerMap.set(data.id, deco)

            // Apply decoration if this is the active editor
            if (isActiveEditor) {
                editor.setDecorations(deco.handle, [data.range])
            }
        }
    }

}
