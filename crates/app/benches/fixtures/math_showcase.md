# Math showcase

A hand-checked note for looking at LaTeX rendering (#13): open it in Jade and compare each formula with what you expect. Lines with tall formulas (matrices, stacked limits) get taller rows; the line you are editing shows its source and shrinks back. Inside `::: columns` and `::: aside` regions rows do not grow yet, so tall formulas there overlap, and `$$` blocks there stay source.

## 1. Plain prose around inline math

The area of a circle is $\pi r^2$, its circumference is $2\pi r$, and Euler's identity $e^{i\pi} + 1 = 0$ ties five constants together. Lorem ipsum dolor sit amet, consectetur adipiscing elit, with $a^2 + b^2 = c^2$ in the middle of a sentence that is long enough to wrap onto a second row when the window is narrow, followed by more filler so the wrap point lands near a formula like $x_1 + x_2 + \dots + x_n$ and then keeps going to the end of the paragraph.

Math at the very start: $E = mc^2$ opens this line. This line ends with math: the golden ratio is $\varphi = \frac{1 + \sqrt{5}}{2}$

- A list item with $a_n = a_{n-1} + d$ inside it
- Another with **bold** text, `code`, and $\sqrt{2}$ side by side
  - A nested item: $\sum_{i=1}^{n} i = \frac{n(n+1)}{2}$
- [ ] A task with $x \ne 0$
- [x] A finished task with $\forall \epsilon > 0$

> A blockquote with $\int_0^1 x\,dx = \tfrac{1}{2}$ in it.
>
> > Nested quote: $\lim_{n \to \infty} \left(1 + \frac{1}{n}\right)^n = e$

> [!note] A callout with math
> The quadratic formula is $x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$.

### A heading with $\alpha + \beta$ in it

## 2. Scripts, fractions, roots

Superscripts and subscripts: $x^2$, $x_i$, $x_i^2$, $x^{2^2}$, $x_{i_j}$, $a^{b^{c^d}}$, ${}^{14}\mathrm{C}$.

Fractions: $\frac{1}{2}$, $\frac{a+b}{c-d}$, $\frac{\frac{1}{x}}{\frac{1}{y}}$, $\dfrac{1}{2}$, $\tfrac{1}{2}$, $\binom{n}{k}$, $a \over b$.

Roots: $\sqrt{x}$, $\sqrt{x^2 + y^2}$, $\sqrt[3]{27}$, $\sqrt[n]{a}$, $\sqrt{\frac{a}{b}}$, $\sqrt{\sqrt{\sqrt{x}}}$.

## 3. Big operators and limits

Inline (limits to the side): $\sum_{k=1}^{n} k^2$, $\prod_{i=1}^{n} i$, $\int_a^b f(x)\,dx$, $\oint_C \vec{F}\cdot d\vec{r}$, $\bigcup_{i} A_i$, $\lim_{x \to 0} \frac{\sin x}{x} = 1$.

Named operators: $\sin x$, $\cos^2 \theta + \sin^2 \theta = 1$, $\log_2 n$, $\ln e = 1$, $\max_{x} f(x)$, $\det A$, $\gcd(a, b)$, $\operatorname{rank} A$.

## 4. Delimiters

$(a + b)$, $[a, b]$, $\{x \mid x > 0\}$, $\langle u, v \rangle$, $|x|$, $\|v\|$, $\lfloor x \rfloor$, $\lceil x \rceil$.

Sized to content: $\left( \frac{a}{b} \right)$, $\left[ \frac{1}{1 + x^2} \right]$, $\left\{ \sum_{i=1}^{n} x_i \right\}$, $\left| \frac{a}{b} \right|$, $\left\langle \frac{u}{v} \right\rangle$, $\left. \frac{df}{dx} \right|_{x=0}$.

Manual sizes: $\big( x \big)$, $\Big( x \Big)$, $\bigg( x \bigg)$, $\Bigg( x \Bigg)$.

## 5. Symbols

Greek lower: $\alpha \beta \gamma \delta \epsilon \varepsilon \zeta \eta \theta \vartheta \iota \kappa \lambda \mu \nu \xi \pi \varpi \rho \varrho \sigma \varsigma \tau \upsilon \phi \varphi \chi \psi \omega$

Greek upper: $\Gamma \Delta \Theta \Lambda \Xi \Pi \Sigma \Upsilon \Phi \Psi \Omega$

Relations: $= \ne \equiv \approx \sim \simeq \cong \propto < > \le \ge \ll \gg \subset \supset \subseteq \supseteq \in \notin \ni \perp \parallel$

Binary operators: $+ - \pm \mp \times \div \cdot \ast \star \circ \bullet \cap \cup \vee \wedge \oplus \otimes \setminus$

Arrows: $\to \leftarrow \leftrightarrow \Rightarrow \Leftarrow \Leftrightarrow \mapsto \uparrow \downarrow \implies \iff$

Misc: $\infty \partial \nabla \forall \exists \nexists \emptyset \neg \angle \hbar \ell \Re \Im \aleph \prime \dots \cdots \vdots \ddots$

## 6. Fonts and text

$\mathbb{R}$, $\mathbb{N}$, $\mathbb{Z}$, $\mathbb{Q}$, $\mathbb{C}$, $\mathcal{L}$, $\mathcal{F}$, $\mathfrak{g}$, $\mathscr{H}$, $\mathbf{v}$, $\boldsymbol{\alpha}$, $\mathrm{d}x$, $\mathsf{T}$, $\mathtt{x}$, $\mathit{diff}$.

Text inside math: $x = 5 \text{ if } y > 0$, $\text{speed} = \frac{\text{distance}}{\text{time}}$, $f(x) = x^2 \quad \text{for all } x$.

Spacing: $a\,b$, $a\;b$, $a\quad b$, $a\qquad b$, $a\!b$, $a\ b$.

## 7. Accents and decorations

$\hat{x}$, $\bar{x}$, $\vec{v}$, $\tilde{n}$, $\dot{x}$, $\ddot{x}$, $\widehat{xyz}$, $\overline{AB}$, $\underline{x}$, $\overbrace{a + b}^{n}$, $\underbrace{a + b}_{m}$, $\overset{!}{=}$, $\xrightarrow{f}$, $\not\in$, $\boxed{x = 1}$.

Strikes: $\cancel{x}$, $\bcancel{x+1}$, $\xcancel{\frac{a}{b}}$, $\frac{\cancel{a}b}{\cancel{a}c}$.

## 8. Environments

Matrices inline: $\begin{pmatrix} a & b \\ c & d \end{pmatrix}$ and $\begin{bmatrix} 1 & 0 \\ 0 & 1 \end{bmatrix}$ and $\begin{vmatrix} a & b \\ c & d \end{vmatrix}$.

Cases: $f(x) = \begin{cases} x & x \ge 0 \\ -x & x < 0 \end{cases}$

Arrays, small matrices and stacked limits: $\begin{array}{c|c} a & b \\ \hline c & d \end{array}$, $\begin{smallmatrix} a & b \\ c & d \end{smallmatrix}$, $\sum_{\substack{i < n \\ j > m}} x_{ij}$, $\begin{aligned} a &= b \\ cc &= d \end{aligned}$.

Wide marks: $\widehat{abcdef}$, $\widetilde{abcdef}$, $\overrightarrow{ABCDEFGH}$, $\xrightarrow[g]{abcdefgh}$, $\overbrace{a + b + c + d + e}^{n}$.

Display blocks (single line):

$$\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$$

$$\sum_{n=0}^{\infty} \frac{x^n}{n!} = e^x$$

Display block over several lines (needs the block wiring, so it stays source for now):

$$
\begin{aligned}
(a + b)^2 &= a^2 + 2ab + b^2 \\
(a - b)^2 &= a^2 - 2ab + b^2
\end{aligned}
$$

## 9. Things that are NOT math (must stay plain text)

- Currency: it costs $5 and then $6 more, so $11 in total.
- A lone dollar sign: $
- Escaped: \$x\$ is not a formula, and neither is a price of \$20.
- Space after the opener: $ x $ stays text.
- Digit right after the closer: $x$5 stays text.
- Inside code: `$x^2$` and a fenced block below.

```
$x^2 + y^2$ in a fence is code, never math.
```

## 10. Broken formulas (must show source or an error, never crash)

- Unclosed group: $\frac{a$
- Unknown command: $\foo{x}$
- Stray brace: $x}$
- Deep nesting: $\frac{\frac{\frac{\frac{\frac{1}{2}}{3}}{4}}{5}}{6}$
- Empty: $$ and $ $ and $\ $

## 11. Mixed markdown with math

| Name | Formula | Note |
|------|---------|------|
| Circle | $\pi r^2$ | area (tables show source for now) |
| Sphere | $\frac{4}{3}\pi r^3$ | volume |
| Normal | $\frac{1}{\sigma\sqrt{2\pi}}e^{-\frac{(x-\mu)^2}{2\sigma^2}}$ | density |

A link to [[Another note]] next to $x^2$, then **bold $x^2$ inside bold**, *italic $y^2$ inside italic*, ~~struck $z^2$~~, and an autolink https://example.com beside $\alpha$.

A very long formula that must wrap or overflow gracefully: $a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_{10} + a_{11} + a_{12} + a_{13} + a_{14} + a_{15} + a_{16} + a_{17} + a_{18} + a_{19} + a_{20} + a_{21} + a_{22} + a_{23} + a_{24} + a_{25}$ and then some trailing text.

---

*End of the showcase. Add new constructs here as the renderer learns them.*

## 12. Display blocks (multi-line `$$`)

A block that parses is drawn centred over rows sized to it; click it or arrow into it to edit the source.

$$
\begin{pmatrix} a & b \\ c & d \end{pmatrix}
\int_0^1 x^2 \, dx
$$

Lines broken with `\\` outside an environment are stacked and centred:

$$
a = b + c \\
d = e
$$

Sized delimiters and phantoms: $\bigl( \frac{a}{b} \bigr)$, $\Bigg[ x \Bigg]$, $a\hspace{1em}b$, $\smash{\frac{x}{y}} + \phantom{z}$, $a \mod n$.

A block that does not parse stays source:

$$
\frac{1
$$

A `[[link]]` inside a block is not a link:

$$
x = [[not a link]]
$$

Colour and numbering: $\textcolor{red}{a} + \color{#0a0} b + c$

$$
E = mc^2 \tag{1}
$$

$$
\textcolor{blue}{\frac{a}{b}} = \color{orange} \sqrt{x} \tag*{(star)}
$$
