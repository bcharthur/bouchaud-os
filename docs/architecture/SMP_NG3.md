# SMP-NG3

SMP-NG3 remplace progressivement les hypothèses NG2 qui rendaient le navigateur
vulnérable aux arrêts globaux.

## État implémenté

* les invalidations TLB sont publiées dans une slot par CPU émetteur et ciblent
  le snapshot `AddressSpace.active_cpus`;
* `munmap` sépare retrait PTE, attente hors BKL et libération des frames;
* les pages demand-pagées portent un état Missing/Loading/Present/Failed et une
  WaitQueue, empêchant deux loaders de matérialiser la même page;
* chaque CpuLocal possède une RunQueue physique protégée par SpinLock; la table
  historique reste la source de metadata pendant la migration et réconcilie les
  anciens wakeups;
* chaque CPU utilise un timer TSC-deadline local lorsque CPUID et le timebase le
  permettent; le broadcast PIT n'est alors plus émis;
* les délais de sommeil et futex reposent sur `monotonic_ns`;
* busy/idle par cœur est mesuré en nanosecondes monotones plutôt que par nombre
  d'IRQ livrées.

## Limite structurelle restante

`Task.process` reste un `Rc<RefCell<Process>>`. Le BKL est donc encore requis
pour les accès généraux au Process et le #PF conserve une section legacy autour
des metadata/VFS. Le retirer correctement exige de séparer les domaines MM,
FD, signaux et identité dans des objets synchronisés distincts; relâcher
simplement le BKL avec un `RefMut` vivant créerait une violation d'aliasing et
un panic RefCell. SMP-NG3 ne prétend pas que cette migration est terminée.
