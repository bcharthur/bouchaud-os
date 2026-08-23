# Accounting CPU et stealing

Chaque Task possède un unique curseur `last_account_ns`, armé seulement lorsque
`on_cpu >= 0`. Entrée/sortie noyau avancent ce curseur sans le désarmer; switch
et blocage l'avancent puis le désarment. Un snapshot ajoute virtuellement la
tranche vivante sans modifier le curseur. Cela évite à la fois la tranche perdue
et son double comptage dans deux fenêtres.

En debug, le delta d'un TID entre deux snapshots ne peut dépasser la fenêtre
monotone de plus d'une milliseconde. Un processus additionne ensuite les TID et
peut atteindre `online_cpus * 100%`; une Task unique reste à 100%.

Le work stealing est un pull d'un CPU sans travail. Une IRQ de quantum prouve au
contraire que la Task courante est encore runnable : elle ne vole plus une Task
distante lorsque sa queue locale est vide. Cet ancien échange conservait tous
les CPU occupés mais permutait inutilement les Tasks à chaque quantum, expliquant
le couple observé « milliers de migrations + équilibre instantané instable ».
