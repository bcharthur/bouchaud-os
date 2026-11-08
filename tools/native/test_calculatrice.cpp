// Cœur EXACT du programme Bouchaud Calculatrice en ring3.
#include <cassert>
#include <climits>
#include <string>
#include "../../userland/apps/calculatrice/calculatrice_core.h"

static void ok(const std::string& input, calc_i64 expected) {
    calc_i64 n = 0;
    assert(calc_execute(input.c_str(), input.size(), &n) == CALC_OK);
    assert(n == expected);
}
static void err(const std::string& input, calc_status status) {
    calc_i64 n = 1;
    assert(calc_execute(input.c_str(), input.size(), &n) == status);
}
int main() {
    ok("ADD 2 3", 5);
    ok("SUB -2 3", -5);
    ok("MUL -3 -9", 27);
    ok("DIV 7 2", 3);
    ok("ADD 9223372036854775807 0", LLONG_MAX);
    ok("ADD -9223372036854775808 0", LLONG_MIN);
    err("ADD 9223372036854775807 1", CALC_OVERFLOW);
    err("SUB -9223372036854775808 1", CALC_OVERFLOW);
    err("MUL 9223372036854775807 2", CALC_OVERFLOW);
    err("DIV -9223372036854775808 -1", CALC_OVERFLOW);
    err("DIV 1 0", CALC_DIV_ZERO);
    err("ADD 1 2 3", CALC_FORMAT);
    err("ADD 999999999999999999999999 1", CALC_FORMAT);
    err("EXEC 2 3", CALC_FORMAT);
    char buf[32];
    assert(std::string(buf, calc_format(LLONG_MIN, buf, sizeof(buf))) == "-9223372036854775808");
    assert(std::string(buf, calc_format(0, buf, sizeof(buf))) == "0");
    return 0;
}
