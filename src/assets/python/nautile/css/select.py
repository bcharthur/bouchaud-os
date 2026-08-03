"""Appariement des selecteurs contre le DOM.

Le test se fait de droite a gauche — le maillon le plus specifique d'abord —
comme dans les moteurs reels : c'est ce qui permet d'eliminer la grande
majorite des regles en un seul test avant de remonter l'arbre.
"""

from ..dom import Element
from .parser import ADJACENT, CHILD, DESCENDANT, SIBLING

# Pseudo-classes considerees comme vraies sans interaction utilisateur. Sans
# ca, une page qui met tout son style dans `:not(...)` ou `:root` perd son
# apparence.
_ALWAYS_TRUE = frozenset({"root", "first-child", "last-child", "only-child",
                          "first-of-type", "last-of-type", "any-link", "link"})

_INTERACTIVE = frozenset({"hover", "active", "focus", "focus-within",
                          "focus-visible", "visited", "target", "checked"})


class MatchContext:
    """Etat dynamique consulte par les pseudo-classes."""

    __slots__ = ("hovered", "focused", "visited")

    def __init__(self, hovered=None, focused=None, visited=None):
        self.hovered = hovered
        self.focused = focused
        self.visited = visited if visited is not None else frozenset()


_DEFAULT_CONTEXT = MatchContext()


def matches(element, selector, context=None):
    """Vrai si `element` correspond a `selector`."""
    parts = selector.parts
    if not parts:
        return False
    if context is None:
        context = _DEFAULT_CONTEXT
    if not _matches_simple(element, parts[-1], context):
        return False
    return _matches_ancestors(element, parts, len(parts) - 1, context)


def _matches_ancestors(element, parts, index, context):
    """Remonte la chaine de combinateurs a partir du maillon `index`."""
    if index == 0:
        return True

    part = parts[index]
    previous = parts[index - 1]
    combinator = part.combinator

    if combinator == CHILD:
        parent = element.parent_node
        if not isinstance(parent, Element):
            return False
        if not _matches_simple(parent, previous, context):
            return False
        return _matches_ancestors(parent, parts, index - 1, context)

    if combinator == ADJACENT:
        sibling = _previous_element(element)
        if sibling is None or not _matches_simple(sibling, previous, context):
            return False
        return _matches_ancestors(sibling, parts, index - 1, context)

    if combinator == SIBLING:
        sibling = _previous_element(element)
        while sibling is not None:
            if _matches_simple(sibling, previous, context) and _matches_ancestors(
                sibling, parts, index - 1, context
            ):
                return True
            sibling = _previous_element(sibling)
        return False

    # Descendant : n'importe quel ancetre peut satisfaire le maillon.
    ancestor = element.parent_node
    while isinstance(ancestor, Element):
        if _matches_simple(ancestor, previous, context) and _matches_ancestors(
            ancestor, parts, index - 1, context
        ):
            return True
        ancestor = ancestor.parent_node
    return False


def _previous_element(node):
    sibling = node.previous_sibling
    while sibling is not None and not isinstance(sibling, Element):
        sibling = sibling.previous_sibling
    return sibling


def _matches_simple(element, part, context):
    if part.tag and part.tag != "*" and element.tag != part.tag:
        return False

    if part.ids:
        element_id = element.attrs.get("id")
        for wanted in part.ids:
            if element_id != wanted:
                return False

    if part.classes:
        classes = element.class_list
        if not classes:
            return False
        for wanted in part.classes:
            if wanted not in classes:
                return False

    for name, operator, value in part.attrs:
        if not _matches_attr(element, name, operator, value):
            return False

    for pseudo in part.pseudo_classes:
        if not _matches_pseudo(element, pseudo, context):
            return False

    return True


def _matches_attr(element, name, operator, value):
    if name not in element.attrs:
        return False
    if not operator:
        return True
    actual = element.attrs[name]
    if operator == "=":
        return actual == value
    if operator == "~=":
        return value in actual.split()
    if operator == "|=":
        return actual == value or actual.startswith(value + "-")
    if operator == "^=":
        return actual.startswith(value)
    if operator == "$=":
        return actual.endswith(value)
    if operator == "*=":
        return value in actual
    return False


def _matches_pseudo(element, pseudo, context):
    if pseudo in _ALWAYS_TRUE:
        if pseudo == "root":
            return element.tag == "html"
        if pseudo in ("first-child", "first-of-type"):
            return _previous_element(element) is None
        if pseudo in ("last-child", "last-of-type"):
            return _next_element(element) is None
        if pseudo == "only-child":
            return _previous_element(element) is None and _next_element(element) is None
        if pseudo in ("link", "any-link"):
            return element.tag == "a" and "href" in element.attrs
        return True

    if pseudo == "hover":
        return context.hovered is element
    if pseudo in ("focus", "focus-visible", "focus-within"):
        return context.focused is element
    if pseudo == "visited":
        href = element.attrs.get("href")
        return href is not None and href in context.visited
    if pseudo in _INTERACTIVE:
        return False

    if pseudo == "not()":
        # On ne resout pas l'argument : refuser ferait disparaitre du style
        # legitime, alors qu'accepter ne fait qu'appliquer un style de plus.
        return True
    if pseudo.endswith("()"):
        return True

    # Pseudo-classe inconnue : on n'invalide pas la regle.
    return True


def _next_element(node):
    sibling = node.next_sibling
    while sibling is not None and not isinstance(sibling, Element):
        sibling = sibling.next_sibling
    return sibling


class RuleIndex:
    """Index des regles par cle du selecteur le plus a droite.

    Sans index, styler une page de 2000 elements avec 3000 regles fait
    6 millions de tests. L'index ramene ca a quelques dizaines par element.
    """

    __slots__ = ("by_id", "by_class", "by_tag", "universal", "count")

    def __init__(self):
        self.by_id = {}
        self.by_class = {}
        self.by_tag = {}
        self.universal = []
        self.count = 0

    def add_stylesheet(self, sheet):
        for rule in sheet.rules:
            for selector in rule.selectors:
                self._add(selector, rule)

    def _add(self, selector, rule):
        entry = (selector, rule)
        self.count += 1
        key = selector.key
        if key is None:
            self.universal.append(entry)
            return
        kind, value = key
        if kind == "#":
            self.by_id.setdefault(value, []).append(entry)
        elif kind == ".":
            self.by_class.setdefault(value, []).append(entry)
        else:
            self.by_tag.setdefault(value, []).append(entry)

    def candidates(self, element):
        """Regles potentiellement applicables a `element`."""
        found = list(self.universal)
        element_id = element.attrs.get("id")
        if element_id:
            found.extend(self.by_id.get(element_id, ()))
        for name in element.class_list:
            found.extend(self.by_class.get(name, ()))
        found.extend(self.by_tag.get(element.tag, ()))
        return found
