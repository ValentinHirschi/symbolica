"""Compare native MathML and highlighted text for nested Symbolica expressions.

Run with a Symbolica build containing the MathML display changes:
    marimo edit examples/mathml_nested_marimo.py
Requires marimo and the optional Symbolica notebook dependencies (anywidget).
"""

import marimo

__generated_with = "0.25.1"
app = marimo.App(width="full")


@app.cell
def _():
    import marimo as mo
    from symbolica import E, S

    return E, S, mo


@app.cell(hide_code=True)
def _(mo):
    mo.md("""
    # Nested expressions: MathML vs. the old view

    Compare the **same expression** in both displays. Choose an example, vary the
    nesting depth, and click the inline markers to open a popup with its own page size
    and **Previous/Next** controls. **Show more here** expands that part in place. **Inspect** opens it in the main view; **Back to parent** returns. Click or Tab into the expression, then use **+ / −** to zoom
    (25–3200%) and **0** to reset. The old view retains its bracket colours and indentation.
    Click `+ [1]` (sum) or `· [2]` (product) markers to explore omitted parts;
    hover over a marker for a preview. Nested detail adapts to the available width;
    each popup has its own pagination controls. The old view is below MathML.
    """)
    return


@app.cell(hide_code=True)
def _(mo):
    example = mo.ui.dropdown(
        options=[
            "Your expression",
            "Nested rational functions",
            "Nested function calls",
            "Second derivative of a composite expression",
            "Derivative notation and negative powers",
            "Large sums inside function arguments",
            "Mixed sum and product abbreviations",
            "Products of sums",
            "Large product inside a function argument",
            "Function with many arguments",
            "Expanded polynomial: 286 terms",
            "Deep nesting: 48 function levels",
        ],
        value="Your expression",
        label="Example",
        full_width=True,
    )
    depth = mo.ui.slider(2, 16, value=6, step=1, label="Nesting depth (first two examples)", show_value=True)
    custom_input = mo.ui.text(
        value="der(0,1,f,x,y)/((a+b)^2*(1+x^2)) + 2*x*y/(3*(1+z)^2)",
        label="Your expression",
        placeholder="Enter Symbolica syntax, e.g. x*y/((a+b)^2*(1+z))",
        full_width=True,
    ).form(submit_button_label="Apply expression", show_clear_button=False)
    mo.vstack([example, custom_input, depth])
    return custom_input, depth, example


@app.cell(hide_code=True)
def _(E, S, depth):
    _rational = "x+y"
    for _k in range(1, depth.value + 1):
        _rational = f"(x+{_k})/(1+(y+{_k})*({_rational}))"

    _functions = "x^2+y^2+z^2"
    for _k in range(1, depth.value + 1):
        _functions = f"F{_k}(x+{_k},G{_k}(y^2+{_k},({_functions})),z^{_k+1})"

    _deep = "x^2+y^2"
    for _k in range(48):
        _deep = f"F{_k}({_deep},x+{_k+1})"

    _left = "+".join(f"({i+1})*x^{i}" for i in range(151))
    _right = "+".join(f"({i+2})*y^{i}" for i in range(121))
    _product_x = "*".join(f"(1+x^{i})" for i in range(1, 41))
    _product_y = "*".join(f"(1+y^{i})" for i in range(1, 33))
    _sums = ["+".join(f"{j+1}*q{i}^{j}" for j in range(24)) for i in range(1, 33)]
    _arguments = ",".join(f"f{i}(x+{i},y^{i})" for i in range(1, 65))
    _composite = E("exp(sin(x^2)/(1+cos(x+y)^2))*log(1+(x+y)^2)/(1+f(x,g(y,z))^2)")
    _x = S("x")

    examples = {
        "Nested rational functions": (
            E(_rational),
            "A continued rational expression. Compare the nested denominator structure and bracket colours.",
            'r = E("x+y")\nfor k in range(1, depth + 1):\n    r = (x+k)/(1+(y+k)*r)',
        ),
        "Nested function calls": (
            E(_functions),
            "Alternating F and G calls, each with several arguments. This tests whether sibling arguments and nesting levels remain legible.",
            'r = "x^2+y^2+z^2"\nfor k in range(1, depth + 1):\n    r = f"F{k}(x+{k},G{k}(y^2+{k},({r})),z^{k+1})"\nexpr = E(r)',
        ),
        "Second derivative of a composite expression": (
            _composite.derivative(_x).derivative(_x),
            "An actual differentiation result mixing trigonometric functions, exponentials, logarithms, unknown functions, and rational factors.",
            'expr = E("exp(sin(x^2)/(1+cos(x+y)^2))*log(1+(x+y)^2)/(1+f(x,g(y,z))^2)")\nexpr.derivative(S("x")).derivative(S("x"))',
        ),
        "Derivative notation and negative powers": (
            E("der(0,1,f,x,y)/(a+b)^2 + der(2,g,x)/(1+x^2)^(3/2) + (a+b)^(-1/2)"),
            "Derivative orders appear above the function name, and negative powers appear as reciprocals. Zoom in to inspect the fractions and superscripts.",
            'E("der(0,1,f,x,y)/(a+b)^2 + der(2,g,x)/(1+x^2)^(3/2) + (a+b)^(-1/2)")',
        ),
        "Large sums inside function arguments": (
            E(f"A(({_left})*(a+b),B(({_right})/(1+z^2),f(x,y,z)))"),
            "The outer expression has two arguments, with sums of 151 and 121 terms. The viewer shows as much nested detail as fits the available width. Hover over a + [1] marker for the next terms, or click it to open a popup with pagination and an Inspect button.",
            'left = sum((i+1)*x**i for i in range(151))\nright = sum((i+2)*y**i for i in range(121))\nexpr = A(left*(a+b), B(right/(1+z**2), f(x,y,z)))',
        ),
        "Expanded polynomial: 286 terms": (
            E("(x+y+z+1)^10").expand(),
            "A complete expansion with 286 terms. The MathML viewer fetches one page at a time; the text preview is limited to 100 terms.",
            'E("(x+y+z+1)^10").expand()',
        ),
        "Mixed sum and product abbreviations": (
            E(f"Mix(({_left}),({_product_x}),({_right})*({_product_y}))"),
            "Three arguments contain a long sum, a long product, and a sum multiplied by a product. Explore the + markers for omitted terms and the middle-dot markers for omitted factors. Show more here expands only the selected part; resize the window to see which details fit.",
            'left = "+".join(f"{i+1}*x^{i}" for i in range(151))\nright = "+".join(f"{i+2}*y^{i}" for i in range(121))\npx = "*".join(f"(1+x^{i})" for i in range(1, 41))\npy = "*".join(f"(1+y^{i})" for i in range(1, 33))\nexpr = E(f"Mix(({left}),({px}),({right})*({py}))")',
        ),
        "Products of sums": (
            E("Factorized(" + "*".join(f"({s})" for s in _sums) + ")"),
            "Factorized contains 32 factors, each a 24-term polynomial in a different variable. A product marker reveals more factors; a sum marker inside a factor reveals more terms. Try expanding one factor in place, then Inspect to explore it separately.",
            'sums = ["+".join(f"{j+1}*q{i}^{j}" for j in range(24)) for i in range(1, 33)]\nexpr = E("Factorized(" + "*".join(f"({s})" for s in sums) + ")")',
        ),
        "Large product inside a function argument": (
            E(f"Amplitude(p,({_product_x}),z)"),
            "Amplitude has three arguments: p, a product of 40 factors, and z. The large middle argument gets a numbered marker while the function name and small sibling arguments remain visible. Click its marker to page through factors, or Show more here to reveal them inside Amplitude.",
            'product = "*".join(f"(1+x^{i})" for i in range(1, 41))\nexpr = E(f"Amplitude(p,({product}),z)")',
        ),
        "Function with many arguments": (
            E(f"Kernel({_arguments})"),
            "One function with 64 arguments. This abbreviates the argument list itself, using a comma before its numbered marker. The popup pages through arguments, rather than terms or factors, and keeps the Kernel function name in view.",
            'arguments = ",".join(f"f{i}(x+{i},y^{i})" for i in range(1, 65))\nexpr = E(f"Kernel({arguments})")',
        ),
        "Deep nesting: 48 function levels": (
            E(_deep),
            "A deliberate stress test. MathML keeps function names visible when abbreviating calls, as F([1]). Click a numbered marker to browse its popup or choose Inspect. The text view shows the full nested structure.",
            'r = "x^2+y^2"\nfor k in range(48):\n    r = f"F{k}({r},x+{k+1})"\nexpr = E(r)',
        ),
    }
    return (examples,)


@app.cell(hide_code=True)
def _(E, custom_input, example, examples, mo):
    if example.value == "Your expression":
        construction = custom_input.value or "der(0,1,f,x,y)/((a+b)^2*(1+x^2)) + 2*x*y/(3*(1+z)^2)"
        _error = None
        try:
            expr = E(construction)
        except Exception as _exception:
            _error = str(_exception)
        mo.stop(_error is not None, mo.callout(mo.md(f"Could not parse the expression:\n\n{_error}"), kind="danger"))
        description = "Edit the input above and click Apply expression. Both views display the same result."
    else:
        expr, description, construction = examples[example.value]
    mathml = expr.formatted()
    legacy = expr.formatted(mathml=False, max_line_length=65, max_terms=100)
    mo.vstack([
        mo.md(f"### {example.value}\n\n{description}"),
        mo.vstack([
            mo.vstack([
                mo.md("**MathML** · `expr.formatted()`"),
                mo.as_html(mathml),
            ]).style({"min-width": "0", "overflow": "auto", "max-height": "75vh", "padding": "12px", "border": "1px solid var(--gray-5)", "border-radius": "8px"}),
            mo.vstack([
                mo.md("**Old view** · `expr.formatted(mathml=False)`"),
                mo.as_html(legacy),
            ]).style({"min-width": "0", "overflow": "auto", "max-height": "75vh", "padding": "12px", "border": "1px solid var(--gray-5)", "border-radius": "8px"}),
        ], align="stretch", gap=1),
        mo.accordion({"How this expression was constructed": mo.md(f"```python\n{construction}\n```")}),
    ])
    return (expr,)


@app.cell
def _(expr):
    expr.formatted()
    return


@app.cell
def _():
    return


if __name__ == "__main__":
    app.run()
