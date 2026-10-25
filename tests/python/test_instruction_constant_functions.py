from symbolica import E, S


def test_exported_instructions_list_registered_constants():
    x = S("x")
    exported = E("x + pi").evaluator([x]).get_instructions()
    ((index, function, tags, arguments),) = exported.constant_functions
    assert function == E("pi")
    assert tags == [] and arguments == []
    assert ("const", index) in exported.instructions[0][2]


def test_exported_instructions_list_fixed_argument_functions():
    x = S("x")
    f = S(
        "constant_functions_square",
        eval={"float": lambda a: a[0] ** 2, "complex": lambda a: a[0] ** 2},
    )
    exported = E("x + constant_functions_square(3)").evaluator([x]).get_instructions()
    ((index, function, tags, arguments),) = exported.constant_functions
    assert function == f
    assert tags == [] and arguments == [E("3")]
    assert ("const", index) in exported.instructions[0][2]


def test_exported_instructions_without_registered_constants():
    x = S("x")
    exported = E("x + 0").evaluator([x]).get_instructions()
    assert exported.constant_functions == []
