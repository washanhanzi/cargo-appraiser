import { ColorThemeKind, Range, TextEditor, TextEditorDecorationType, Uri, window } from "vscode"
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

export class DecorationCtrl {
    map: Map<string, Map<string, Decoration>> = new Map()

    listen(client: LanguageClient) {
        client.onRequest("textDocument/decoration/create", async (params, next) => {
            this.create(Uri.parse(params.uri), params.id, params.text, params.range, params.kind)
            return
        })

        client.onRequest("textDocument/decoration/updateRange", async (params, next) => {
            this.updateRange(
                Uri.parse(params.uri),
                params.id,
                params.range,
            )
            return
        })

        client.onRequest("textDocument/decoration/delete", async (params, next) => {
            this.delete(
                Uri.parse(params.uri),
                params.id,
            )
            return
        })

        client.onRequest("textDocument/decoration/reset", async (params, next) => {
            this.reset(Uri.parse(params.uri))
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

    create(uri: Uri, id: string, text: string, range: Range, kind: string) {
        const color = config.getColor(kind)

        const deco: Decoration = {
            text,
            color,
            margin: '0 0 0 4em', // Add some margin to the left
            range,
            handle: window.createTextEditorDecorationType({
                after: {
                    contentText: text,
                    color,
                    margin: '0 0 0 4em' // Add some margin to the left
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

    delete(uri: Uri, id: string) {
        const innerMap = this.map.get(uri.path)
        if (!innerMap) {
            return
        }
        innerMap.delete(id)
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
}
