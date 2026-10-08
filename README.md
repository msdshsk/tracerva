# Tracerva

**Raster-to-SVG tracing in Rust.**

ロゴ・装飾・題字などのラスター画像を、編集可能なSVGパスへ変換するRustライブラリです。
減色、色むらの整理、輪郭抽出、共有境界のベジェ曲線近似、円・凸多角形の幾何補正を提供します。
画像処理に `image`、エラー定義に `thiserror` を使用します。

現在のバージョンは **0.1.0** です。APIは今後変更される可能性があります。

## 実行例

Rust 2024 edition対応のツールチェーンが必要です。

```sh
cargo run --release --locked --example trace -- input.png output.svg
```

PNG・JPEG・WebPを読み込み、最大16色に減色し、幾何補正と入力画像上の2pxの曲線平滑化を適用します。
この例では背景を保持します。用途に応じた色・背景の設定はライブラリAPIから指定できます。

## ライブラリとして使う

ローカルのプロジェクトから参照する例です。crates.ioへの公開はまだ行っていません。

```toml
[dependencies]
tracerva = { path = "../tracerva" }
image = { version = "0.25", default-features = false, features = ["png"] }
```

白背景の白黒題字を、背景なしのSVGへ変換する例:

```rust,no_run
use tracerva::{Options, RefineOptions, refine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image = image::open("title.png")?.to_rgba8();
    let result = refine(
        &image,
        &Options::default(),
        &RefineOptions {
            palette: vec![[0, 0, 0], [255, 255, 255]],
            background: Some([255, 255, 255]),
            geometry: true,
            smooth: 2.0,
            ..Default::default()
        },
    )?;
    std::fs::write("title.svg", result.svg)?;
    Ok(())
}
```

## 主な設定

| 設定 | 用途 |
| --- | --- |
| `Options::colors` | 自動減色の最大色数（1〜256、既定16） |
| `RefineOptions::palette` | 指定した色への直接割り当て |
| `merge_distance` | 近い色をRGB距離で統合。固定パレットとは併用不可 |
| `min_region_area` | 小さな前景領域を隣接領域へ統合。既定0で無効 |
| `background` | 指定色に近い領域を省略。内部の抜きにも適用 |
| `smooth` | 入力画素単位の平滑化の強さ。既定0で無効 |
| `geometry` | 独立した閉輪郭の円・凸多角形を認識して補正 |
| `silhouette` | 背景以外を一色として扱う。背景指定が必須 |
| `paint: Paint::Outline` | 塗りなし・ガイド線付きの閉じたパスを出力 |

`refine` の既定設定では曲線近似・幾何補正は無効です。
基本APIの `trace` は画素境界に基づく多角形を出力します。
曲線近似・幾何補正を使う場合、`Options::tolerance` は既定の0にしてください。
詳しい設定は `cargo doc --no-deps --open` で参照できます。

## 適用範囲と制約

- 主対象はフラットなロゴ、装飾、十分な解像度の題字です。写真やグラデーションの自動復元は対象外です。
- 平滑化や微小領域の整理によって、細い線・狭い隙間・小さな点が変化することがあります。接続や穴の保持を保証するものではありません。
- 幾何補正は認識条件に合う独立円・凸多角形に限ります。意図的な角落としを補正する可能性もあります。
- 線の中心線や編集可能なフォント文字を復元する機能ではありません。出力は輪郭のパスです。
- 入力のアルファは指定マット色（既定は白）へ合成します。元画像の半透明は保持しません。背景省略による透明化は可能です。
- SVGはRGBです。CMYK変換や印刷入稿用PDFの生成は行いません。
- 入力上限は1,600万画素です。必要メモリ・処理時間は画像の複雑さにも依存します。

## グレースケールで形を抽出して着色（実験）

`refine_grayscale` はカラーの違いで直接パスを分けず、グレースケールの領域から形を作った後、
元画像の代表色を各パスに塗り戻すモードです。通常の `refine` とは別APIで、既存の動作は維持しています。

1. マット色へ合成した画像から明るさを算出し、メディアンフィルターで細かなノイズを抑える。
2. 明るさのヒストグラムを、階調内の二乗誤差が小さくなるよう指定階調数に分割する。
3. 微小な連結領域を隣接する大きな領域へ整理し、共有境界の曲線近似・幾何補正でパスを作る。
4. 各連結領域の元RGB値からチャンネルごとの中央値を取り着色する。境界から1画素内側を優先し、内側がなければ領域全体を使う。

```sh
cargo run --release --locked --example grayscale -- input.png output.svg 8 2 16
```

末尾の引数は順に階調数・ノイズ除去半径・微小領域の面積です。省略時も `8 / 2 / 16` です。
例では白に近い領域を省略し、幾何補正と2pxの曲線近似を適用します。
ライブラリでは `GrayscaleOptions` と既存の `RefineOptions` を指定します。
RGBの色統合・固定パレット・シルエット・既存の微小領域整理設定とは併用しません。

階調数は最終的な色数ではありません。同じ明るさでも離れた領域には別々の色が付きます。
逆に、接している同じ明るさの色は一つの領域になり、元の色境界は復元できません。
階調数を増やすとグラデーションや陰影まで領域に分かれることがあります。
ノイズ除去と微小領域整理は細い線や小さな装飾も変えるため、必要なら0にして比較してください。
グラデーション・半透明の自動復元は行いません。

## WASM

コアライブラリはファイルI/Oや外部プロセスを使用しません。
`wasm32-unknown-unknown` 向けのコンパイルチェックを実施しています。
`wasm/` にJavaScript向けのバインディング、`web/` にTypeScriptのブラウザデモがあります。
デモはWeb Workerで変換し、画像の読み込み・比較・SVG保存をブラウザ内で完結します。

```sh
rustup target add wasm32-unknown-unknown
cargo check --lib --target wasm32-unknown-unknown --locked
```

## Webデモ

Node.js 24、Rust stable、wasm-pack 0.15.0を使用します。

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --version 0.15.0 --locked
cd web
npm ci
npm run dev
```

本番用は `npm run build`、ローカル確認は `npm run preview` です。
ビルド時にRustからWASMとJavaScriptの呼び出し口を生成し、TypeScriptの型チェック後に
`web/dist/` へ静的サイトを出力します。生成物はGit管理しません。

デモでは画像のドラッグ＆ドロップ、自動減色・白黒・指定パレット・グレースケール経由の切り替え、
色数・平滑化・幾何補正・背景省略・塗りなし出力の設定、
倍率とスクロールを揃えた比較、SVG保存ができます。サンプル画像はブラウザ内で描画します。
入力はPNG・JPEG・WebP、最大1,600万画素・30MBです。端末の性能によって処理時間やメモリ使用量が変わります。

### GitHub Pages

[`.github/workflows/pages.yml`](.github/workflows/pages.yml) で、PR時にRustのテスト・静的検査と
WASM・TypeScriptのビルドを実行します。`main` へのpush時は同じ検証後、GitHub Pagesへ自動デプロイします。
Actions画面からの手動実行にも対応します。

リポジトリの **Settings → Pages → Build and deployment → Source** を **GitHub Actions** に設定してください。
このリポジトリの公開先は `https://msdshsk.github.io/tracerva/` です。
ビルドは相対URLを使うため、リポジトリ名のサブパスでも配信できます。
Pagesへの書き込み権限はデプロイジョブだけに付与し、PRではデプロイしません。

## 開発

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
```

画像生成素材・比較用の他社エンジン・比較レポートはこのリポジトリに含めていません。

## 参考資料

輪郭トレース・曲線近似の設計にあたり、以下の資料を参考にしました。

- Peter Selinger, [Potrace: a polygon-based tracing algorithm](https://potrace.sourceforge.net/potrace.pdf)
- Philip J. Schneider, *An Algorithm for Automatically Fitting Digitized Curves* — [Graphics Gemsの参考実装](https://github.com/erich666/GraphicsGems/blob/master/gems/FitCurves.c)

## ライセンス

Copyright (c) 2026 msd.shsk

Tracervaは **MIT OR Apache-2.0** の二重ライセンスで提供します。
利用者は [MIT License](LICENSE-MIT) または [Apache License 2.0](LICENSE-APACHE) のいずれかを選択できます。
依存クレートにはそれぞれのライセンスが適用されます。
