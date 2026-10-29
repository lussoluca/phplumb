<?php
namespace App;

use Lib\Labelled;

function enum_interfaces(\UnitEnum $unit, \BackedEnum $backed, Labelled $labelled): void
{
    $unit->/*@broken*/nowhere();
    $backed->/*@broken*/nowhere();
    $labelled->/*@broken*/nowhere();
    $unit->cases();
    $backed->from('x');
    $labelled->cases();
}
