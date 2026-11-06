/* Bouchaud Calculatrice — cœur pur, freestanding, partagé avec tests hôte.
 * Cette tranche n'a aucune dépendance au noyau/GUI ni à la libc.
 */
#ifndef BO_CALCULATRICE_CORE_H
#define BO_CALCULATRICE_CORE_H
typedef long long calc_i64;
typedef unsigned long long calc_u64;
typedef unsigned long calc_size;
enum calc_status { CALC_OK = 0, CALC_FORMAT = 1, CALC_OVERFLOW = 2, CALC_DIV_ZERO = 3 };

static int calc_blanc(char c) { return c == ' ' || c == '\n' || c == '\t' || c == '\r'; }
static void calc_skip(const char *s, calc_size len, calc_size *at) {
    while (*at < len && calc_blanc(s[*at])) (*at)++;
}
static int calc_entier(const char *s, calc_size len, calc_size *at, calc_i64 *out) {
    calc_u64 magnitude = 0;
    calc_u64 limite;
    int negatif = 0, digits = 0;
    calc_skip(s, len, at);
    if (*at < len && (s[*at] == '-' || s[*at] == '+')) {
        negatif = s[*at] == '-'; (*at)++;
    }
    limite = negatif ? 9223372036854775808ULL : 9223372036854775807ULL;
    while (*at < len && s[*at] >= '0' && s[*at] <= '9') {
        calc_u64 digit = (calc_u64)(s[*at] - '0');
        if (magnitude > (limite - digit) / 10) return 0;
        magnitude = magnitude * 10 + digit;
        (*at)++;
        digits++;
    }
    if (!digits) return 0;
    if (negatif && magnitude == 9223372036854775808ULL)
        *out = (-9223372036854775807LL - 1);
    else
        *out = negatif ? -(calc_i64)magnitude : (calc_i64)magnitude;
    return 1;
}
static enum calc_status calc_execute(const char *s, calc_size len, calc_i64 *out) {
    calc_size at = 0;
    char op[3];
    calc_i64 a, b;
    calc_skip(s, len, &at);
    if (at + 3 >= len) return CALC_FORMAT;
    op[0] = s[at]; op[1] = s[at + 1]; op[2] = s[at + 2]; at += 3;
    if (!calc_blanc(s[at++])) return CALC_FORMAT;
    if (!calc_entier(s, len, &at, &a) || !calc_entier(s, len, &at, &b)) return CALC_FORMAT;
    calc_skip(s, len, &at);
    if (at != len) return CALC_FORMAT;
    if (op[0]=='A' && op[1]=='D' && op[2]=='D')
        return __builtin_add_overflow(a,b,out) ? CALC_OVERFLOW : CALC_OK;
    if (op[0]=='S' && op[1]=='U' && op[2]=='B')
        return __builtin_sub_overflow(a,b,out) ? CALC_OVERFLOW : CALC_OK;
    if (op[0]=='M' && op[1]=='U' && op[2]=='L')
        return __builtin_mul_overflow(a,b,out) ? CALC_OVERFLOW : CALC_OK;
    if (op[0]=='D' && op[1]=='I' && op[2]=='V') {
        if (b == 0) return CALC_DIV_ZERO;
        if (a == (-9223372036854775807LL - 1) && b == -1) return CALC_OVERFLOW;
        *out = a / b; return CALC_OK;
    }
    return CALC_FORMAT;
}
static calc_size calc_format(calc_i64 val, char *buf, calc_size cap) {
    char digits[22]; calc_size n=0, out=0;
    calc_u64 mag = val < 0 ? (calc_u64)(-(val + 1)) + 1 : (calc_u64)val;
    if (!cap) return 0;
    do { digits[n++] = (char)('0' + (mag % 10)); mag /= 10; } while (mag);
    if (val < 0 && out < cap) buf[out++] = '-';
    while (n && out < cap) buf[out++] = digits[--n];
    return out;
}
#endif
